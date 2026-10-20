//! The client room pass: at the start
//! of the client update, before the unit update, each room of the client
//! act's active-room list (list order) whose flags bit 0 is clear gets
//! its critters, then its client
//! presets, then bit 0 := 1. Every unit is made by
//! the client creator ([`super::objects::create_client_unit`]), so the
//! pass sets the client GUIDs of the arrival. The critter AI
//! runs in the C monsters' walk ([`c_monsters`]).
//!
//! Not run: the level-8 pass (with the level-8 flag set);
//! D2RUST has no model of that flag.

use d2_sim::rng::Seed;

use super::drlg::DrlgRoomId;
use super::objects::create_client_unit;
use super::world::{
    ClientUnit, ClientWorld, KindData, ModelInputs, MonsterData, UnitKey, MONSTER, PLAYER,
};

/// The size-query mask of the critter placement.
const PLACE_MASK: u16 = 0x3F11;

/// The room pass. No client DRLG:
/// nothing.
pub fn room_pass(w: &mut ClientWorld, inputs: &ModelInputs) {
    let Some(d) = w.drlg.as_ref() else {
        return;
    };
    let rooms = d.active_rooms();
    if rooms.iter().all(|r| d.populated(r.room)) {
        return;
    }
    // The footprints the placement test reads (PROVISIONAL).
    super::client_missiles::stamp_unit_footprints(w, &inputs.tables.monsters);
    super::client_missiles::stamp_object_footprints(w, &inputs.objclient.rows);
    for r in rooms {
        let Some(d) = w.drlg.as_ref() else {
            return;
        };
        if d.populated(r.room) {
            continue;
        }
        critters(w, inputs, r.room, r.level);
        presets(w, inputs, r.room);
        if let Some(d) = w.drlg.as_mut() {
            d.set_populated(r.room);
        }
    }
}

/// One room seed draw `rand(n)`: n < 1 → 0
/// with no draw.
fn rand(w: &mut ClientWorld, room: DrlgRoomId, n: i32) -> i32 {
    w.drlg
        .as_mut()
        .and_then(|d| d.room_seed(room))
        .map_or(0, |s| s.roll(n) as i32)
}

/// The critter pass with (room).
fn critters(w: &mut ClientWorld, inputs: &ModelInputs, room: DrlgRoomId, level: u16) {
    let Some(l) = inputs.tables.levels.get(usize::from(level)) else {
        return;
    };
    let c = l.critters;
    for i in 0..4 {
        let class = c.cmon[i];
        if class < 0 {
            return;
        }
        // `pct`: one step, lo % 100 (unsigned), drawn for every slot.
        if rand(w, room, 100) >= i32::from(c.cpct[i]) {
            continue;
        }
        let row = inputs
            .tables
            .monsters
            .get(class as usize)
            .copied()
            .flatten();
        let (min, max) = row.map_or((0, 0), |m| (i32::from(m.min_grp), i32::from(m.max_grp)));
        let mut n = rand(w, room, max - min) + min;
        if c.camt[i] != 0 {
            n *= i32::from(c.camt[i]);
        }
        for _ in 0..n {
            place_critter(w, inputs, room, class);
        }
    }
}

/// `place_critter` with (room, c): up to 10 random
/// points of the room (x drawn first); the first free one gets the
/// critter. 10 occupied points: no critter and no GUID.
fn place_critter(w: &mut ClientWorld, inputs: &ModelInputs, room: DrlgRoomId, class: i16) {
    let Some(row) = inputs
        .tables
        .monsters
        .get(class as usize)
        .copied()
        .flatten()
    else {
        return;
    };
    let size = i32::from(row.size_x);
    let Some(d) = w.drlg.as_ref() else {
        return;
    };
    let Some(a) = d.drlg.active_room(room) else {
        return;
    };
    let (id, r) = (a.id, a.subtiles);
    for _ in 0..10 {
        let x = r.x + rand(w, room, r.w - 1);
        let y = r.y + rand(w, room, r.h - 1);
        if super::client_missiles::size_query(w, id, x, y, size, PLACE_MASK) != 0 {
            continue;
        }
        let Some(key) = create_client_unit(w, MONSTER, class as u32, x as u16, y as u16) else {
            return;
        };
        // The create links the unit into its room's list (the recache):
        // the draw order files
        // it from there.
        w.room_units.place(super::world::view_key(key), Some(room));
        // The 0xAC create: the monster set-up (path, footprint,
        // mode 1 NU, first frame, direction).
        super::critter_path::setup(w, inputs, key, id, x, y);
        return;
    }
}

/// The client presets with (room): each
/// preset unit of the room with flag bit 0, in list order, at its
/// position plus the room's sub-tile origin, with its type and mode.
fn presets(w: &mut ClientWorld, inputs: &ModelInputs, room: DrlgRoomId) {
    let Some(d) = w.drlg.as_ref() else {
        return;
    };
    let Some(a) = d.drlg.active_room(room) else {
        return;
    };
    let (x0, y0) = (a.subtiles.x, a.subtiles.y);
    let list = d.client_presets(room);
    for p in list {
        let (x, y) = ((p.x + x0) as u16, (p.y + y0) as u16);
        let Some(key) = create_client_unit(w, p.unit_type as u8, p.class as u32, x, y) else {
            continue;
        };
        // The object create's static-path set-up links it into the list
        // of the room at its cell (head insert): the draw's room-list
        // walk lists it.
        let room = w.room_at(x, y).map(|r| r.room);
        w.room_units.place(super::world::view_key(key), room);
        if let Some(u) = w.objclient.set_c.get_mut(&key) {
            u.mode = p.mode;
            // The object init of a client preset runs the animation
            // set-up in its mode (at the Rogue Encampment arrival: frame
            // count and speed set on every client object).
            if u.key.unit_type == super::world::OBJECT {
                if let Some(row) = inputs.objclient.rows.get(u.class as usize) {
                    let _ = super::objects::anim_setup(u, row, p.mode);
                    // The same init sets the flat flag 0x100000 when
                    // `OrderFlag[mode]` = 1: the
                    // River1–3 presets (classes 40–42) draw in the shadow
                    // pass.
                    if row.order_flag.get(p.mode as usize) == Some(&1) {
                        u.flag_flat = true;
                    }
                }
            }
        }
    }
}

/// The C monsters' walk (after the S units): each
/// still in set C runs the critter AI.
pub fn c_monsters(w: &mut ClientWorld, inputs: &ModelInputs) {
    let mut walk = super::objects::CWalk::new(MONSTER);
    while let Some(key) = walk.next(w) {
        // The unit update (→ the monster
        // update), then the critter AI.
        if w.objclient.set_c.contains_key(&key) {
            super::critter_path::update(w, inputs, key);
        }
        if w.objclient.set_c.contains_key(&key) {
            critter_ai(w, inputs, key);
        }
    }
}

/// Monster mode 0 and 12 are the dead modes the slot scan skips.
fn alive_player(u: &ClientUnit) -> bool {
    u.mode != 0 && u.mode != 12 && u.position.is_some()
}

/// The monster size of a class (`monstats2` `SizeX`); 0 without a row.
fn monster_size(inputs: &ModelInputs, class: u32) -> i32 {
    inputs
        .tables
        .monsters
        .get(class as usize)
        .and_then(|c| c.as_ref())
        .map_or(0, |c| i32::from(c.size_x))
}

fn think_mut(w: &mut ClientWorld, key: UnitKey) -> Option<&mut MonsterData> {
    match &mut w.objclient.set_c.get_mut(&key)?.kind {
        KindData::Monster(d) => Some(d),
        _ => None,
    }
}

/// One step of U's seed (+0x20); a unit without a seed reads 0.
fn unit_step(w: &mut ClientWorld, key: UnitKey) -> u32 {
    let Some(u) = w.objclient.set_c.get_mut(&key) else {
        return 0;
    };
    let Some((lo, hi)) = u.seed else {
        return 0;
    };
    let mut s = Seed::new(lo, hi);
    let r = s.step();
    u.seed = Some((s.lo, s.hi));
    r
}

/// The critter AI with (U).
fn critter_ai(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    let Some(d) = think_mut(w, key) else {
        return;
    };
    let t = d.think;
    d.first_think.get_or_insert(t);
    if t > 0 {
        d.think = t - 1;
        return;
    }
    let u = &w.objclient.set_c[&key];
    let class = u.class;
    let size = monster_size(inputs, class);
    // P, D: the nearest of the player slots (first one on a tie).
    let mut nearest: Option<((i32, i32), i32)> = None;
    for (k, p) in &w.units {
        if k.unit_type != PLAYER || !alive_player(p) {
            continue;
        }
        let dist = super::objects::distance(u, size, p, 2);
        if nearest.is_none_or(|(_, n)| dist < n) {
            nearest = Some((cell_i32(p.cell()), dist));
        }
    }
    let (p, dist) = match nearest {
        Some((p, dist)) => (p, dist),
        None if w.local().is_none() => {
            set_think(w, key, 25);
            request(w, inputs, key, Request::Idle);
            return;
        }
        None => ((0, 0), i32::MAX),
    };
    if dist > 30 {
        set_think(w, key, (dist - 15).min(200));
        request(w, inputs, key, Request::Idle);
        return;
    }
    // From here P exists (D ≤ 30).
    match class {
        CHICKEN | BUG => chicken_ai(w, inputs, key),
        RAT | SCORPION | LARVA | MINISPIDER => rat_ai(w, inputs, key, p, dist),
        BAT => bat_ai(w, inputs, key, p, dist),
        // The zoo test never applies to a critter: the
        // `zoo` column of `monstats.txt` is empty for every class `levels.txt`
        // `cmon1`..`cmon4` names (chicken, rat, bat, bug, scorpion, larva,
        // snake, parrot, minispider, bunny).
        // Birds 157 / 158, maggot 227, worldstone effect 574 and the
        // fetish shamans 278..=282 are in no `cmon` column, so they are
        // never critters and run nothing here. The snake 318 body runs
        // (`snake_ai`). PROVISIONAL (settled by a draws comparison with a
        // parrot in the Act 3 jungle and a bunny at the Act 5 siege): the bodies
        // of parrot 319 and bunny 556 are specified there but not
        // modelled; they run nothing.
        157 | 158 | 319 | 227 | 556 | 574 | 278..=282 => {}
        SNAKE => snake_ai(w, inputs, key, p, dist),
        _ => request(w, inputs, key, Request::Idle),
    }
}

const CHICKEN: u32 = 149;
const BUG: u32 = 268;
const RAT: u32 = 151;
const BAT: u32 = 159;
const SCORPION: u32 = 269;
const LARVA: u32 = 283;
const MINISPIDER: u32 = 339;
const SNAKE: u32 = 318;

/// ``: one step of U's seed, `lo % 100` (unsigned).
fn r100(w: &mut ClientWorld, key: UnitKey) -> u32 {
    unit_step(w, key) % 100
}

/// U's path cell (its position without a path).
fn own_cell(w: &ClientWorld, key: UnitKey) -> (i32, i32) {
    super::critter_path::cell(w, key).unwrap_or_else(|| {
        w.objclient
            .set_c
            .get(&key)
            .map_or((0, 0), |u| cell_i32(u.cell()))
    })
}

/// The critter flag F (monster data +0x34).
fn flag_mut(w: &mut ClientWorld, key: UnitKey) -> Option<&mut i32> {
    think_mut(w, key).map(|d| &mut d.critter_flag)
}

/// `away(V, d, code)`: s := sign(U − V) per axis; the
/// cell U + s blocked (size 1, U's move mask) → no
/// request (0); else request `code` at U + d·s (1).
fn away(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    v: (i32, i32),
    d: i32,
    code: u8,
    r4: i32,
) -> bool {
    let (ux, uy) = own_cell(w, key);
    let s = ((ux - v.0).signum(), (uy - v.1).signum());
    let mask = super::critter_path::move_mask(w, key).unwrap_or(0);
    let id = w.room_at(ux as u16, uy as u16).and_then(|r| {
        let d = w.drlg.as_ref()?;
        d.drlg.active_room(r.room).map(|a| a.id)
    });
    let blocked = id.is_none_or(|id| {
        super::client_missiles::size_query(w, id, ux + s.0, uy + s.1, 1, mask) != 0
    });
    if blocked {
        return false;
    }
    let (x, y) = (ux + d * s.0, uy + d * s.1);
    request(w, inputs, key, Request::To { code, x, y, r4 });
    true
}

/// The body of rat 151, scorpion 269, larva 283 and minispider 339.
fn rat_ai(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, p: (i32, i32), d: i32) {
    let mode = w.objclient.set_c[&key].mode;
    if matches!(mode, 0 | 12) {
        return;
    }
    if d < 2 && own_cell(w, key) == p {
        // `own`: code 8 (death) at U's position.
        // PROVISIONAL (settled by a state capture of a rat or scorpion
        // run over by the player): the death request's hover,
        // state-mask and SplEndDeath parts are not run for a C monster:
        // mode set 0 (death) with the tail only.
        super::critter_path::mode_only(w, inputs, key, 0, 0);
        return;
    }
    if mode != 1 {
        return;
    }
    if d < 4 {
        away(w, inputs, key, p, 6, 1, 0);
        return;
    }
    let v = if d > 7 {
        scan_dead(w, inputs, key)
    } else {
        None
    };
    let f = flag_mut(w, key).map_or(0, |f| *f);
    if let (Some(v), 0) = (v, f) {
        // `toward(V, 6)`: code 0 on V (walk to the unit).
        // PROVISIONAL (settled by a state capture of a rat walking to a
        // dead unit): the critter path has no unit target; the walk
        // goes to V's cell.
        request(
            w,
            inputs,
            key,
            Request::To {
                code: 1,
                x: v.0,
                y: v.1,
                r4: 6,
            },
        );
        if let Some(f) = flag_mut(w, key) {
            *f = 1;
        }
        return;
    }
    if f != 0 {
        if r100(w, key) < 20 {
            if let Some(f) = flag_mut(w, key) {
                *f = 0;
            }
        }
        set_think(w, key, 32);
        request(w, inputs, key, Request::Idle);
        return;
    }
    if r100(w, key) < 75 {
        set_think(w, key, 30);
        request(w, inputs, key, Request::Idle);
    } else {
        step(w, inputs, key, 5, 1, 0);
    }
}

/// The rat body's scan with the filter: in the
/// rooms near U, a player in mode 0 / 0x11 or a monster in mode 0 / 12
/// (not U) with 3 ≤ dist(U, W) ≤ 25 and dist(local player, W) > 7 takes
/// one draw of U's seed (`lo & 1`); 1 → V := W, the last accepted wins.
/// Units in S by key, then set C (as [`nearest_near`]).
fn scan_dead(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) -> Option<(i32, i32)> {
    let u = &w.objclient.set_c[&key];
    let size = monster_size(inputs, u.class);
    let d = w.drlg.as_ref()?;
    let room = w.room_at(u.cell().0, u.cell().1)?;
    let rects: Vec<_> = d
        .adjacency(room.room)
        .into_iter()
        .filter_map(|r| d.drlg.active_room(r).map(|a| a.subtiles))
        .collect();
    let local = w.local();
    let mut found: Vec<(i32, i32)> = Vec::new();
    for (k, o) in w.units.iter().chain(w.objclient.set_c.iter()) {
        if *k == key {
            continue;
        }
        let (dead, other_size) = match k.unit_type {
            PLAYER => (matches!(o.mode, 0 | 0x11), 2),
            MONSTER => (matches!(o.mode, 0 | 12), monster_size(inputs, o.class)),
            _ => continue,
        };
        let Some(pos) = o.position else {
            continue;
        };
        let (px, py) = cell_i32(pos);
        if !dead || !rects.iter().any(|r| r.contains(px, py)) {
            continue;
        }
        let dist = super::objects::distance(u, size, o, other_size);
        let far_from_local =
            local.is_none_or(|l| super::objects::distance(l, 2, o, other_size) > 7);
        if (3..=25).contains(&dist) && far_from_local {
            found.push((px, py));
        }
    }
    let mut v = None;
    for w_cell in found {
        if unit_step(w, key) & 1 == 1 {
            v = Some(w_cell);
        }
    }
    v
}

/// The bat body 159.
fn bat_ai(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, p: (i32, i32), d: i32) {
    if w.objclient.set_c[&key].mode != 1 {
        return;
    }
    if d < 5 {
        if r100(w, key) < 66 {
            away(w, inputs, key, p, 8, 1, 30);
        } else {
            step(w, inputs, key, 12, 1, 30);
        }
    } else {
        step(w, inputs, key, 9, 1, 0);
    }
}

/// The snake body 318: dead (mode 0 or
/// 12) returns; standing on P is `own` (code 8, death); then only in mode
/// 1: D ≤ 1 `away(P, 4, 1, 0)`, else `` < 75 waits (T = 32, idle),
/// else `step(4, 1, 0)`.
fn snake_ai(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, p: (i32, i32), d: i32) {
    let mode = w.objclient.set_c[&key].mode;
    if matches!(mode, 0 | 12) {
        return;
    }
    if own_cell(w, key) == p {
        super::critter_path::mode_only(w, inputs, key, 0, 0);
        return;
    }
    if mode != 1 {
        return;
    }
    if d <= 1 {
        away(w, inputs, key, p, 4, 1, 0);
    } else if r100(w, key) < 75 {
        set_think(w, key, 32);
        request(w, inputs, key, Request::Idle);
    } else {
        step(w, inputs, key, 4, 1, 0);
    }
}

fn set_think(w: &mut ClientWorld, key: UnitKey, t: i32) {
    let t = w.tick_rate.ticks(t);
    if let Some(d) = think_mut(w, key) {
        d.think = t;
    }
}

/// A critter's mode request.
enum Request {
    /// Code 7 at U's own position.
    Idle,
    /// Code 1 (walk to a point, mode 2) or 0x0C (mode 8 S1) at (x, y)
    /// with the record's.
    To { code: u8, x: i32, y: i32, r4: i32 },
}

/// The request on a C monster ([`super::critter_path`]): code 7 at the
/// own position, code 1 to the point (the client path). Code 0x0C (a
/// point-group code, mode 8 S1): face the point, mode set 8, and with
/// the class's `S1mv` bit the path to the point.
fn request(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, r: Request) {
    match r {
        Request::Idle => super::critter_path::idle(w, inputs, key),
        Request::To { code: 1, x, y, r4 } => super::critter_path::walk_to(w, inputs, key, x, y, r4),
        Request::To { x, y, r4, .. } => {
            super::critter_path::point_mode(w, inputs, key, 8, x, y, r4)
        }
    }
}

/// `step(d, code)`: from U's path cell, x ± d by bit 0
/// of one step of U's seed, then y ± d by bit 0 of the next, then the
/// request.
fn step(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, d: i32, code: u8, r4: i32) {
    let (x, y) = super::critter_path::cell(w, key).unwrap_or_else(|| {
        w.objclient
            .set_c
            .get(&key)
            .map_or((0, 0), |u| cell_i32(u.cell()))
    });
    let sx = if unit_step(w, key) & 1 != 0 { -d } else { d };
    let sy = if unit_step(w, key) & 1 != 0 { -d } else { d };
    let (x, y) = (x + sx, y + sy);
    request(w, inputs, key, Request::To { code, x, y, r4 });
}

/// `chicken_ai(U)`.
fn chicken_ai(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    let u = &w.objclient.set_c[&key];
    if u.mode != 1 {
        return;
    }
    let size = monster_size(inputs, u.class);
    let Some((v, dist)) = nearest_near(w, inputs, key, size) else {
        return;
    };
    if dist < 4 {
        // Flee from V, move mask 0xC01.
        let (ux, uy) = cell_i32(u.cell());
        let s = ((ux - v.0).signum(), (uy - v.1).signum());
        let room = w.room_at(u.cell().0, u.cell().1);
        let id = room.and_then(|r| {
            let d = w.drlg.as_ref()?;
            d.drlg.active_room(r.room).map(|a| a.id)
        });
        let free = id.is_some_and(|id| {
            super::client_missiles::size_query(w, id, ux + s.0, uy + s.1, 1, 0xC01) == 0
        });
        // Free: to U + 4s; else step(4, 0x0C, 120).
        if free {
            let (x, y) = (ux + 4 * s.0, uy + 4 * s.1);
            request(
                w,
                inputs,
                key,
                Request::To {
                    code: 0x0C,
                    x,
                    y,
                    r4: 120,
                },
            );
        } else {
            step(w, inputs, key, 4, 0x0C, 120);
        }
    } else if unit_step(w, key) % 100 < 30 {
        // step(2, 1, 0): walk.
        step(w, inputs, key, 2, 1, 0);
    } else {
        set_think(w, key, 5);
    }
}

fn cell_i32((x, y): (u16, u16)) -> (i32, i32) {
    (i32::from(x), i32::from(y))
}

/// The nearest unit in the rooms near U
/// (U's room's adjacency array): a player, or a monster not class 149
/// or 556, alive, not U; path distance (first one on a tie, S units by
/// key, then set C). Returns its cell and the distance.
fn nearest_near(
    w: &ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    size: i32,
) -> Option<((i32, i32), i32)> {
    let u = &w.objclient.set_c[&key];
    let d = w.drlg.as_ref()?;
    let room = w.room_at(u.cell().0, u.cell().1)?;
    let rects: Vec<_> = d
        .adjacency(room.room)
        .into_iter()
        .filter_map(|r| d.drlg.active_room(r).map(|a| a.subtiles))
        .collect();
    let mut best: Option<((i32, i32), i32)> = None;
    let units = w.units.iter().chain(w.objclient.set_c.iter());
    for (k, o) in units {
        if *k == key || o.is_dead() {
            continue;
        }
        let other_size = match k.unit_type {
            PLAYER => 2,
            MONSTER if !matches!(o.class, CHICKEN | 556) => monster_size(inputs, o.class),
            _ => continue,
        };
        let Some(p) = o.position else {
            continue;
        };
        let (px, py) = cell_i32(p);
        if !rects.iter().any(|r| r.contains(px, py)) {
            continue;
        }
        let dist = super::objects::distance(u, size, o, other_size);
        if best.is_none_or(|(_, b)| dist < b) {
            best = Some(((px, py), dist));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::MonsterClass;

    fn inputs() -> ModelInputs {
        let mut i = ModelInputs::default();
        i.tables.monsters = vec![None; 150];
        i.tables.monsters[149] = Some(MonsterClass {
            size_x: 1,
            ..MonsterClass::default()
        });
        i
    }

    /// A world with one chicken in set C at (100, 100) and, when
    /// `player` is given, the local player there.
    fn world(player: Option<(u16, u16)>) -> (ClientWorld, UnitKey) {
        let mut w = ClientWorld::default();
        let key = create_client_unit(&mut w, MONSTER, CHICKEN, 100, 100).unwrap();
        w.objclient.set_c.get_mut(&key).unwrap().mode = 1;
        if let Some(p) = player {
            let pk = UnitKey::new(PLAYER, 1);
            let mut u = ClientUnit::new(pk);
            u.position = Some(p);
            u.mode = 1;
            w.units.insert(pk, u);
            w.local_player = Some(pk);
        }
        (w, key)
    }

    fn think(w: &ClientWorld, key: UnitKey) -> (i32, Option<i32>) {
        match &w.objclient.set_c[&key].kind {
            KindData::Monster(d) => (d.think, d.first_think),
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_new_critter_reads_t_zero_and_a_running_timer_counts_down() {
        let (mut w, key) = world(None);
        let i = inputs();
        // No player and no local player: T = 25, idle (mode 1 stays).
        c_monsters(&mut w, &i);
        assert_eq!(think(&w, key), (25, Some(0)));
        c_monsters(&mut w, &i);
        assert_eq!(think(&w, key), (24, Some(0)));
        assert_eq!(w.objclient.set_c[&key].mode, 1);
    }

    #[test]
    fn a_far_player_sets_the_timer_to_the_distance_less_15_at_most_200() {
        let i = inputs();
        // Δ = 40 on x: (2·(40 − 1) + 0) / 2 = 39 > 30 → T = 24.
        let (mut w, key) = world(Some((140, 100)));
        c_monsters(&mut w, &i);
        assert_eq!(think(&w, key).0, 24);
        let (mut w, key) = world(Some((1000, 100)));
        c_monsters(&mut w, &i);
        assert_eq!(think(&w, key).0, 200);
        // Near (no client DRLG: no unit near U): nothing, no seed draw.
        let (mut w, key) = world(Some((110, 100)));
        let seed = w.objclient.set_c[&key].seed;
        c_monsters(&mut w, &i);
        assert_eq!(think(&w, key).0, 0);
        assert_eq!(w.objclient.set_c[&key].seed, seed);
    }

    /// One critter of `class` in set C at (100, 100), mode 1, and the
    /// local player at `player`.
    fn world_of(class: u32, player: (u16, u16)) -> (ClientWorld, UnitKey, ModelInputs) {
        let mut i = inputs();
        i.tables.monsters = vec![None; class as usize + 1];
        i.tables.monsters[class as usize] = Some(MonsterClass {
            size_x: 1,
            ..MonsterClass::default()
        });
        let mut w = ClientWorld::default();
        let key = create_client_unit(&mut w, MONSTER, class, 100, 100).unwrap();
        let c = w.objclient.set_c.get_mut(&key).unwrap();
        c.mode = 1;
        c.seed = Some((0x1234_5678, 666));
        let pk = UnitKey::new(PLAYER, 1);
        let mut u = ClientUnit::new(pk);
        u.position = Some(player);
        u.mode = 1;
        w.units.insert(pk, u);
        w.local_player = Some(pk);
        (w, key, i)
    }

    #[test]
    fn a_rat_with_the_player_8_to_30_away_rolls_r100_then_rests_or_steps_5() {
        // D = 10 (> 7): no dead unit to scan (no client DRLG), F = 0:
        // < 75 → T = 30 and idle; else step(5, 1, 0): two more draws.
        let (mut w, key, i) = world_of(RAT, (111, 100));
        c_monsters(&mut w, &i);
        let mut s = Seed::new(0x1234_5678, 666);
        let r = s.step() % 100;
        if r < 75 {
            assert_eq!(think(&w, key).0, 30);
        } else {
            s.step();
            s.step();
            assert_eq!(think(&w, key).0, 0);
        }
        assert_eq!(w.objclient.set_c[&key].seed, Some((s.lo, s.hi)));
        // A critter not in mode 1 does nothing (no draw).
        let (mut w, key, i) = world_of(RAT, (111, 100));
        w.objclient.set_c.get_mut(&key).unwrap().mode = 2;
        c_monsters(&mut w, &i);
        assert_eq!(w.objclient.set_c[&key].seed, Some((0x1234_5678, 666)));
    }

    #[test]
    fn a_rat_with_flag_f_set_rests_32_and_may_clear_it() {
        let (mut w, key, i) = world_of(RAT, (111, 100));
        if let KindData::Monster(d) = &mut w.objclient.set_c.get_mut(&key).unwrap().kind {
            d.critter_flag = 1;
        }
        c_monsters(&mut w, &i);
        let r = Seed::new(0x1234_5678, 666).step() % 100;
        let f = match &w.objclient.set_c[&key].kind {
            KindData::Monster(d) => d.critter_flag,
            _ => unreachable!(),
        };
        assert_eq!((think(&w, key).0, f), (32, i32::from(r >= 20)));
    }

    #[test]
    fn a_bat_steps_9_when_the_player_is_5_or_more_away() {
        // D = 10: step(9, 1, 0), two draws of U's seed (x sign, y sign).
        let (mut w, key, i) = world_of(BAT, (111, 100));
        c_monsters(&mut w, &i);
        let mut s = Seed::new(0x1234_5678, 666);
        s.step();
        s.step();
        assert_eq!(w.objclient.set_c[&key].seed, Some((s.lo, s.hi)));
        // D < 5: first (away or step 12), so at least one draw.
        let (mut w, key, i) = world_of(BAT, (103, 100));
        c_monsters(&mut w, &i);
        assert_ne!(w.objclient.set_c[&key].seed, Some((0x1234_5678, 666)));
    }

    #[test]
    fn a_snake_with_the_player_away_rolls_r100_then_waits_32_or_steps_4() {
        // D = 10: < 75 → T = 32 and idle; else step(4, 1, 0): two
        // more draws of U's seed.
        let (mut w, key, i) = world_of(SNAKE, (111, 100));
        c_monsters(&mut w, &i);
        let mut s = Seed::new(0x1234_5678, 666);
        let r = s.step() % 100;
        if r < 75 {
            assert_eq!(think(&w, key).0, 32);
        } else {
            s.step();
            s.step();
            assert_eq!(think(&w, key).0, 0);
        }
        assert_eq!(w.objclient.set_c[&key].seed, Some((s.lo, s.hi)));
        // Not in mode 1 (and the player not on it): nothing, no draw.
        let (mut w, key, i) = world_of(SNAKE, (111, 100));
        w.objclient.set_c.get_mut(&key).unwrap().mode = 2;
        c_monsters(&mut w, &i);
        assert_eq!(w.objclient.set_c[&key].seed, Some((0x1234_5678, 666)));
    }

    #[test]
    fn a_snake_the_player_stands_on_dies_and_a_dead_one_does_nothing() {
        // On P: `own`, request code 8 (death), no seed draw.
        let (mut w, key, i) = world_of(SNAKE, (100, 100));
        c_monsters(&mut w, &i);
        assert_eq!(w.objclient.set_c[&key].mode, 0);
        assert_eq!(w.objclient.set_c[&key].seed, Some((0x1234_5678, 666)));
        // Dead (mode 0): returns before the on-P test and any draw.
        let (mut w, key, i) = world_of(SNAKE, (111, 100));
        w.objclient.set_c.get_mut(&key).unwrap().mode = 0;
        c_monsters(&mut w, &i);
        assert_eq!(w.objclient.set_c[&key].seed, Some((0x1234_5678, 666)));
    }

    #[test]
    fn without_a_client_drlg_the_room_pass_makes_nothing() {
        let mut w = ClientWorld::default();
        room_pass(&mut w, &inputs());
        assert!(w.objclient.set_c.is_empty());
        assert_eq!(w.objclient.next_guid, 1);
    }

    #[test]
    fn the_view_tells_a_set_c_monster_from_a_set_s_one_of_the_same_guid() {
        let (mut w, key) = world(None);
        // A server monster with the critter's GUID.
        let server = ClientUnit::new(key);
        w.units.insert(key, server);
        let vk = crate::bridge::world::view_key(key);
        assert_ne!(vk, key);
        let c = w.view_unit(&vk).unwrap();
        assert_eq!((c.key, c.class), (vk, CHICKEN));
        assert_eq!(w.view_unit(&key).unwrap().class, 0);
        let all: Vec<_> = w.view_units().iter().map(|u| u.key).collect();
        assert_eq!(all, [key, vk]);
        // Only monsters of set C are drawn units.
        assert!(w
            .view_unit(&UnitKey::new(2, 5 | crate::bridge::world::VIEW_CLIENT_BIT))
            .is_none());
    }
    #[test]
    fn chicken_think_period_lasts_one_second_at_each_rate() {
        for hz in [25, 60] {
            let (mut w, key) = world(None);
            w.tick_rate = d2_sim::tick::TickRate::new(hz).unwrap();
            c_monsters(&mut w, &inputs());
            assert_eq!(think(&w, key).0, hz as i32);
            for _ in 0..hz {
                c_monsters(&mut w, &inputs());
            }
            assert_eq!(think(&w, key).0, 0);
        }
    }
}
