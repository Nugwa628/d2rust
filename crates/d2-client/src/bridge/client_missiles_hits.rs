//! The client hit functions (`pCltHitFunc`, table), called by
//! the end with the missile
//! m and the hit unit U (none for a wall, a landing or a forced end).
//! `false` (a 0 result) keeps m alive and skips its hit sound and
//! explosion.

use super::super::dispatch::HandlerError;
use super::super::world::{ClientWorld, UnitKey, ITEM, MISSILE, MONSTER, OBJECT, PLAYER};
use super::{cell_of, create_from, flag, rnd, seed_step, ClientMissile, CreateRecord, Env};

/// Ring RX / RY (constants), i = 0…15.
const RING_X: [i32; 16] = [0, 1, 2, 2, 2, 2, 2, 1, 0, -1, -2, -2, -2, -2, -2, -1];
const RING_Y: [i32; 16] = [2, 2, 2, 1, 0, -1, -2, -2, -2, -2, -2, -1, 0, 1, 2, 2];
/// Shard DX8 / DY8 and patterns.
const DX8: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
const DY8: [i32; 8] = [1, 1, 0, -1, -1, -1, 0, 1];
const SHARD_PATTERNS: [&[i32]; 3] = [&[0, 1, 4, 5], &[0, 2, 4, 6], &[0, 3, 5]];
/// Ring of 8 R8X / R8Y (Constants).
const R8_X: [i32; 8] = [0, 2, 2, 2, 0, -2, -2, -2];
const R8_Y: [i32; 8] = [2, 2, 0, -2, -2, -2, 0, 2];
/// Meteor M5X / M5Y and the 18-point F18X / F18Y.
const M5_X: [i32; 5] = [0, 4, 0, -4, 0];
const M5_Y: [i32; 5] = [0, 0, 4, 0, -4];
const F18_X: [i32; 18] = [2, -2, 0, 0, -3, 0, 3, -1, 1, -1, 2, -4, -3, -1, 0, 1, 3, 4];
const F18_Y: [i32; 18] = [
    -2, -2, 2, 5, 3, 3, 3, 2, 1, -1, -1, -2, -2, -3, -4, -3, -3, -2,
];
/// Hit 1's class 265 `fireexplosion2`, hit 31's 344 / 345, rocks' 456,
/// hit 18's light radius 12.
const FIRE_EXPLOSION_2: i32 = 265;
const ICE_BREAK_SMALL: i32 = 344;
const ICE_BREAK_LARGE: i32 = 345;
const FLYING_ROCKS: u32 = 456;
const METEOR_LIGHT: i32 = 12;
/// Hit 9's fixed class 204 `teethexplode`.
const TEETH_EXPLODE: u32 = 204;

/// The hit function `h` of m (0 < h < 81 with a non-null slot; the
/// null slots and h ≤ 0 call nothing and read as non-zero).
///
/// The bodies not modelled yet read as their known result without their
/// effects: 0 on a unit for 36, 37, 38, 41, 57, 61 (and 40 unless its
/// owner test passes), else non-zero (explosion, light, removal); 53 as
/// non-zero. The bodies of 37, 38 and 57 each return 0 when a unit is
/// given. PROVISIONAL: the bodies of 36, 40, 41 and 61 are not known
/// yet; settled by porting each.
pub(super) fn call(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    u: Option<UnitKey>,
    h: i16,
) -> Result<bool, HandlerError> {
    let Some(mm) = w.objclient.missiles.get(&m).copied() else {
        return Ok(true);
    };
    let class = w.objclient.set_c.get(&m).map_or(0, |x| x.class);
    let Some(row) = env.rows.get(class as usize).copied() else {
        return Ok(true);
    };
    let [h1, h2, h3, h4] = row.clt_hit_sub.map(i32::from);
    let [c1, c2, c3] = row.c_hit_par;
    let (x, y) = cell_of(&mm);
    match h {
        // B7 1: disc of class 265 (H1 only gates).
        1 => {
            if h1 >= 0 {
                super::bodies::disc(w, env, m, c1, FIRE_EXPLOSION_2, c2, 1)?;
            }
        }
        // B7 2: ring(m, H1, c1, c2, c3).
        2 => {
            if h1 >= 0 {
                ring(w, env, m, h1, c1, c2, c3)?;
            }
        }
        // B7 3: spawn(H1); H2 ≥ 0 → X := spawn(range pick(H2, H3)), its
        // dead flag set.
        3 => {
            if h1 >= 0 {
                super::spawn(w, env, m, h1 as u32)?;
            }
            if h2 >= 0 {
                let c = if h2 >= h3 {
                    h2
                } else {
                    h2 + rnd(w, m, h3 - h2 + 1)
                };
                if let Some(k) = super::spawn(w, env, m, c as u32)? {
                    if let Some(km) = w.objclient.missiles.get_mut(&k) {
                        km.flat = true;
                    }
                }
            }
        }
        // B12 9: holy bolt. U none → 1. c1 ≠ 0, O given and O may target
        // U as an ally → (the overlay, not modelled as hit 10's) 1. Else U
        // must pass the type test (a player: c2 = 0; a monster: demon when
        // c2 = 2, else undead) or m flies on; passed: H1 ≥ 0 → class 204
        // with flags 0 (start = origin m), owner O, origin m.
        9 => {
            let Some(u) = u else {
                return Ok(true);
            };
            if c1 != 0 && mm.owner.is_some() && ally_target(w, u)? {
                return Ok(true);
            }
            let passed = match u.unit_type {
                PLAYER => c2 == 0,
                MONSTER => {
                    let class = w
                        .units
                        .get(&u)
                        .and_then(|x| env.monsters.get(x.class as usize))
                        .and_then(|c| c.as_ref());
                    class.is_some_and(|c| if c2 == 2 { c.demon } else { c.undead })
                }
                _ => false,
            };
            if !passed {
                return Ok(false);
            }
            if h1 >= 0 {
                let rec = CreateRecord {
                    owner: mm.owner,
                    origin: Some(m),
                    class: TEETH_EXPLODE,
                    skill: mm.skill,
                    level: mm.level,
                    ..CreateRecord::default()
                };
                create_from(w, env, m, &rec)?;
            }
        }
        // B12 12: the fire patch of H1, H2, H3 (up to the first negative):
        // k := c1 > 0 ? c1: max(eval(calc1), 1); v := the missiles
        // evaluator on `SHitCalc1`, ≤ 0 → `Range` + `LevRange` × (level −
        // 1); fire patch(m, list, n, k, v, c2).
        12 => {
            let t = env.skills.ok_or(HandlerError::Invalid(
                "missiles/client-bodies-2.md §B12 hit 12 needs the skills tables",
            ))?;
            let Some(skill) = usize::try_from(mm.skill).ok().and_then(|s| t.skills.get(s)) else {
                return Ok(true);
            };
            let list: Vec<i32> = [h1, h2, h3].into_iter().take_while(|&c| c >= 0).collect();
            if list.is_empty() {
                return Ok(true);
            }
            let owner = mm.owner;
            let k = if c1 > 0 {
                c1
            } else {
                let mut units = super::super::passive::ClientSkills::new(w, owner.unwrap_or(m));
                d2_sim::skills::levels::eval_skill(
                    &mut units,
                    t,
                    owner,
                    skill.calc1,
                    mm.skill,
                    mm.level,
                )
                .max(1)
            };
            let mut v = {
                let mut units = super::super::passive::ClientSkills::new(w, owner.unwrap_or(m));
                d2_sim::skills::levels::eval_missile(
                    &mut units,
                    t,
                    Some(m),
                    owner,
                    row.s_hit_calc1,
                    class as i32,
                    mm.level,
                )
            };
            if v <= 0 {
                v = i32::from(row.range) + i32::from(row.lev_range) * (mm.level - 1);
            }
            fire_patch(w, env, m, &list, k, v, c2)?;
        }
        // B12 53: H1 ≥ 0 → X := H1 with flags 0x21 from m's position to
        // m's target point (owner O, m's skill and level); made → its
        // motion record bounces c1 times by c2 percent, a
        // timed arc of height 0 over c3 ticks, then flag 2
        // cleared.
        53 => {
            if h1 >= 0 {
                let rec = CreateRecord {
                    flags: flag::POSITION | flag::TARGET_ABSOLUTE,
                    owner: mm.owner,
                    class: h1 as u32,
                    x,
                    y,
                    tx: mm.target_point.0,
                    ty: mm.target_point.1,
                    skill: mm.skill,
                    level: mm.level,
                    ..CreateRecord::default()
                };
                if let Some(c) = create_from(w, env, m, &rec)? {
                    if let Some(cm) = w.objclient.missiles.get_mut(&c) {
                        // The base game's arc ORs flag 2 in and the arc end
                        // clears it again.
                        cm.motion.bounce(c1, c2);
                        cm.motion.timed_arc(0, c3);
                        cm.motion.untimed();
                    }
                }
            }
        }
        // B7 10: U given, `ProgOverlay` > 0 → the overlay with (U,
        // ProgOverlay, 2, 0, 0, 0, 0, 0): a kind-2 overlay drawn once (no `1ofN`
        // roll).
        10 => {
            if let (Some(u), Some(inputs)) = (u, env.model) {
                if row.prog_overlay > 0 {
                    super::super::state_overlays::create(
                        w,
                        inputs,
                        u,
                        i32::from(row.prog_overlay),
                        2,
                        [0; 3],
                    );
                }
            }
        }
        // B7 14: spawn(H1); H2 ≥ 0: two seed steps (pattern lo' mod 3,
        // turn lo' & 7); per entry e0: e := (e0 + b) & 7, X := spawn(H2)
        // facing (x + DX8[e], y + DY8[e]).
        14 => {
            if h1 >= 0 {
                super::spawn(w, env, m, h1 as u32)?;
            }
            if h2 >= 0 {
                let a = (seed_step(w, m) % 3) as usize;
                let b = (seed_step(w, m) & 7) as i32;
                for &e0 in SHARD_PATTERNS[a] {
                    let e = ((e0 + b) & 7) as usize;
                    if let Some(c) = super::spawn(w, env, m, h2 as u32)? {
                        face(w, c, (x_of(w, c), (x + DX8[e], y + DY8[e])));
                    }
                }
            }
        }
        // B7 19: c := H1, or H1 + rnd(H2 − H1 + 1) when H2 > H1; spawn(c).
        19 => {
            if h1 >= 0 {
                let c = if h2 > h1 {
                    h1 + rnd(w, m, h2 - h1 + 1)
                } else {
                    h1
                };
                super::spawn(w, env, m, c as u32)?;
            }
        }
        // B7 24: U given and H1 ≥ 0 → spawn(H1) and 0 (m flies on).
        24 => {
            if u.is_some() && h1 >= 0 {
                super::spawn(w, env, m, h1 as u32)?;
                return Ok(false);
            }
        }
        // B7 29: H1 ≥ 0, O given, m's skill in the table: n := c1, 0 →
        // max(eval(calc1), 5); H1 created with flags 1 (0x8001, frames n,
        // when n > 0), skill := m's level (Edge case 1), facing m's.
        29 => {
            let Some(o) = mm.owner else {
                return Ok(true);
            };
            if h1 < 0 {
                return Ok(true);
            }
            let t = env.skills.ok_or(HandlerError::Invalid(
                "missiles/client-bodies.md §B7 hit 29 needs the skills tables",
            ))?;
            let Some(skill) = usize::try_from(mm.skill).ok().and_then(|s| t.skills.get(s)) else {
                return Ok(true);
            };
            let mut n = c1;
            if n == 0 {
                let mut units = super::super::passive::ClientSkills::new(w, o);
                n = d2_sim::skills::levels::eval_skill(
                    &mut units,
                    t,
                    Some(o),
                    skill.calc1,
                    mm.skill,
                    mm.level,
                )
                .max(5);
            }
            let mut rec = CreateRecord {
                flags: flag::POSITION,
                owner: Some(o),
                class: h1 as u32,
                x,
                y,
                skill: mm.level,
                level: mm.level,
                ..CreateRecord::default()
            };
            if n > 0 {
                rec.flags |= flag::RANGE;
                rec.range = n;
            }
            if let Some(c) = create_from(w, env, m, &rec)? {
                set_direction(w, c, mm.direction);
            }
        }
        // B7 31: U given → 0. Else the ice break of m's class (272 → 344;
        // 273, 274, 417 → 345; others −1) at (x, y), facing
        // m's, animation speed 0x80.
        31 => {
            if u.is_some() {
                return Ok(false);
            }
            let c = match class {
                272 => ICE_BREAK_SMALL,
                273 | 274 | 417 => ICE_BREAK_LARGE,
                _ => -1,
            };
            let rec = CreateRecord {
                flags: flag::POSITION,
                owner: mm.owner,
                class: c as u32,
                x,
                y,
                skill: mm.skill,
                level: mm.level,
                ..CreateRecord::default()
            };
            if let Some(c) = create_from(w, env, m, &rec)? {
                if let Some(cm) = w.objclient.missiles.get_mut(&c) {
                    cm.direction = mm.direction;
                    cm.anim_speed = 0x80;
                }
            }
        }
        // B12 39 (`queendeathglob` 354): one step of m's seed; lo odd → 1
        // with nothing. Else a second step: X := spawn(m's class + 1 +
        // (lo & 1)) (355 / 356 `queendeathsplat1`/`2`), X made → it faces
        // m's direction with path flag 0x40.
        39 => {
            if seed_step(w, m) & 1 != 0 {
                return Ok(true);
            }
            let c = class + 1 + (seed_step(w, m) & 1);
            if let Some(c) = super::spawn(w, env, m, c)? {
                if let Some(cm) = w.objclient.missiles.get_mut(&c) {
                    cm.direction = mm.direction;
                    cm.keep_facing = true;
                }
            }
        }
        // B7 44: U given → 0. H1 ≥ 0 → X := spawn(H1) facing m's, its d28
        // = m's (path flag 0x40: the facing stays).
        44 => {
            if u.is_some() {
                return Ok(false);
            }
            if h1 >= 0 {
                if let Some(c) = super::spawn(w, env, m, h1 as u32)? {
                    if let Some(cm) = w.objclient.missiles.get_mut(&c) {
                        cm.direction = mm.direction;
                        cm.d28 = mm.d28;
                    }
                }
            }
        }
        // B7 55: O given: H1, H2, H3 in turn, each ≥ 0 created with flags
        // 0 (start = target = origin m's position), owner O, origin m.
        55 => {
            if let Some(o) = mm.owner {
                for c in [h1, h2, h3] {
                    if c >= 0 {
                        let rec = CreateRecord {
                            owner: Some(o),
                            origin: Some(m),
                            class: c as u32,
                            skill: mm.skill,
                            level: mm.level,
                            ..CreateRecord::default()
                        };
                        create_from(w, env, m, &rec)?;
                    }
                }
            }
        }
        // B12 18: meteor explosion.
        18 => meteor_hit(w, env, m, &mm, [h1, h2, h3, h4], [c1, c2, c3])?,
        // B12 13: guided arrow / bone spirit.
        13 => return guided_hit(w, env, m, &mm, u),
        // B12 16: chain lightning.
        16 => chain(w, env, m, &mm, u, c1)?,
        // B12 25: lightning fury.
        25 => fury(w, env, m, &mm, h1, [c1, c2, c3])?,
        // B12 26: fist of the heavens.
        26 => return fist(w, env, m, &mm, &row, h1, c1),
        // B12 28: ring of 8 around U (else m), step max(c1, 1).
        28 => {
            if let (true, Some(o)) = (h1 >= 0, mm.owner) {
                ring_of_8(w, env, m, o, u.unwrap_or(m), h1, c1.max(1))?;
            }
        }
        // B12 30: the frozen orb's 64 / k novas, each with (d28, d2C) its
        // offset.
        30 => {
            if let (true, Some(o)) = (h1 >= 0, mm.owner) {
                let k = c1.max(1) as usize;
                for i in (0..64).step_by(k) {
                    let (ox, oy) = super::bodies::orb_offset(i);
                    let rec = CreateRecord {
                        flags: flag::TARGET_RELATIVE,
                        owner: Some(o),
                        origin: Some(m),
                        class: h1 as u32,
                        tx: ox,
                        ty: oy,
                        skill: mm.skill,
                        level: mm.level,
                        ..CreateRecord::default()
                    };
                    if let Some(c) = create_from(w, env, m, &rec)? {
                        if let Some(cm) = w.objclient.missiles.get_mut(&c) {
                            (cm.d28, cm.d2c) = (ox, oy);
                        }
                    }
                }
            }
        }
        // B12 52: molten boulder.
        52 => {
            // U given: a monster whose `monstats2` row has `large`
            // (monstats2 flag 11; no row reads 0) bursts m, any
            // other U keeps it.
            if let Some(u) = u {
                let large = u.unit_type == MONSTER
                    && w.units
                        .get(&u)
                        .and_then(|v| env.monsters.get(v.class as usize))
                        .and_then(|c| c.as_ref())
                        .is_some_and(|c| c.large);
                if !large {
                    return Ok(false);
                }
            }
            if h1 >= 0 {
                let rec = CreateRecord {
                    flags: flag::POSITION,
                    owner: mm.owner,
                    class: h1 as u32,
                    x,
                    y,
                    skill: mm.skill,
                    level: mm.level,
                    ..CreateRecord::default()
                };
                create_from(w, env, m, &rec)?;
            }
            rocks(w, env, m, &mm, c1, c2, h2, h3)?;
        }
        // B12 54: U given → 0; else spawn facing(m, H1).
        54 => {
            if h1 >= 0 {
                if u.is_some() {
                    return Ok(false);
                }
                if let Some(c) = super::spawn(w, env, m, h1 as u32)? {
                    set_direction(w, c, mm.direction);
                }
            }
        }
        // B12 56: O given: H1 created with flags 0 (owner O, origin m),
        // facing m's.
        56 => {
            if let (true, Some(o)) = (h1 >= 0, mm.owner) {
                let rec = CreateRecord {
                    owner: Some(o),
                    origin: Some(m),
                    class: h1 as u32,
                    skill: mm.skill,
                    level: mm.level,
                    ..CreateRecord::default()
                };
                if let Some(c) = create_from(w, env, m, &rec)? {
                    set_direction(w, c, mm.direction);
                }
            }
        }
        // Not modelled: the RE-read result. With U given, 36,
        // 37, 38, 41,
        // 57 and 61 return 0 (m flies on); 40
        // returns 0 unless its owner test passes (a player
        // owner, U a monster, U's pet owner = the owner's +0xC),
        // which the model cannot read: 0.
        36 | 37 | 38 | 40 | 41 | 57 | 61 if u.is_some() => return Ok(false),
        _ => {}
    }
    Ok(true)
}

fn set_direction(w: &mut ClientWorld, k: UnitKey, d: u8) {
    if let Some(m) = w.objclient.missiles.get_mut(&k) {
        m.direction = d;
    }
}

/// The precise position of a missile.
fn x_of(w: &ClientWorld, k: UnitKey) -> (u32, u32) {
    w.objclient.missiles.get(&k).map_or((0, 0), |m| m.pos)
}

/// With (path, x, y, 0): the facing toward the centre of sub-tile
/// `to` from `from` (the direction of the vector).
fn face(w: &mut ClientWorld, k: UnitKey, (from, to): ((u32, u32), (i32, i32))) {
    let Some(t) = super::path_tables() else {
        return;
    };
    let centre = |c: i32| ((c as u32) << 16) | 0x8000;
    let (_, d) = d2_sim::path::walk::geom::direction_vector(t, from, (centre(to.0), centre(to.1)));
    set_direction(w, k, d & 63);
}

/// Ring with (m, c, a, b, loops): c = 0
/// → nothing. Flags 0x17 (start (x, y), relative target, velocity given
/// unshifted), loops > 0 → 0x1F with R+0x34 := loops; owner O, origin m,
/// m's skill and level. Velocity := c's `Param1` << 7 for i = 0, k, 2k, …
/// < 16 (k = max(b, 1)); then with a > 0 `Param2` << 7 for i = 1, 1 + a,
/// … ≤ 15; each aims at (RX[i], RY[i]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn ring(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    c: i32,
    a: i32,
    b: i32,
    loops: i32,
) -> Result<(), HandlerError> {
    if c == 0 {
        return Ok(());
    }
    let Some(mm) = w.objclient.missiles.get(&m).copied() else {
        return Ok(());
    };
    // A class outside the table reads a null row in the base game (no live
    // caller); here its parameters read 0 and the create makes none.
    let params = usize::try_from(c)
        .ok()
        .and_then(|c| env.rows.get(c))
        .map_or([0, 0], |r| r.param);
    let (x, y) = cell_of(&mm);
    let mut rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE | flag::VELOCITY | flag::VELOCITY_FIXED,
        owner: mm.owner,
        origin: Some(m),
        class: c as u32,
        x,
        y,
        skill: mm.skill,
        level: mm.level,
        ..CreateRecord::default()
    };
    if loops > 0 {
        rec.flags |= flag::LOOPS;
        rec.loops = loops;
    }
    rec.velocity = params[0] << 7;
    for i in (0..16).step_by(b.max(1) as usize) {
        (rec.tx, rec.ty) = (RING_X[i], RING_Y[i]);
        create_from(w, env, m, &rec)?;
    }
    if a > 0 {
        rec.velocity = params[1] << 7;
        for i in (1..16).step_by(a as usize) {
            (rec.tx, rec.ty) = (RING_X[i], RING_Y[i]);
            create_from(w, env, m, &rec)?;
        }
    }
    Ok(())
}

/// Ring of 8 with (O, P, c, s, l, step):
/// c outside the table → none. Flags 0x1F, owner O, origin P,
/// start pos(P), m's skill and level, loops R+0x34 := level − 1, velocity
/// = c's `Param1` << 7; for i = 0, step, … < 8: relative target (R8X[i],
/// R8Y[i]).
fn ring_of_8(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    o: UnitKey,
    p: UnitKey,
    c: i32,
    step: i32,
) -> Result<(), HandlerError> {
    let Some(crow) = usize::try_from(c).ok().and_then(|c| env.rows.get(c)) else {
        return Ok(());
    };
    let Some(mm) = w.objclient.missiles.get(&m).copied() else {
        return Ok(());
    };
    let at = match w.objclient.missiles.get(&p) {
        Some(pm) => cell_of(pm),
        None => w.units.get(&p).map_or((0, 0), |u| {
            let (x, y) = u.cell();
            (i32::from(x), i32::from(y))
        }),
    };
    let mut rec = CreateRecord {
        flags: flag::POSITION
            | flag::TARGET_RELATIVE
            | flag::VELOCITY
            | flag::LOOPS
            | flag::VELOCITY_FIXED,
        owner: Some(o),
        origin: Some(p),
        class: c as u32,
        x: at.0,
        y: at.1,
        skill: mm.skill,
        level: mm.level,
        loops: mm.level - 1,
        velocity: crow.param[0] << 7,
        ..CreateRecord::default()
    };
    for i in (0..8).step_by(step.max(1) as usize) {
        (rec.tx, rec.ty) = (R8_X[i], R8_Y[i]);
        create_from(w, env, m, &rec)?;
    }
    Ok(())
}

/// Hit 18 (meteor centre): the
/// skills row of m's skill required. H1 ≥ 0: c1 × H1 (flags 0x4001, no
/// light) at (x, y) + (M5X, M5Y)[i mod 5]. H2 ≥ 0: H2 (flags 0x8001,
/// frames := level ≥ 1 ? skills `Param3` + (level − 1) × `Param4`: 0) at
/// (x, y), its light radius 12. Then flags 0xC001 with those frames (0
/// when H2 < 0): H3 ≥ 0 → c2 × H3 at F18[i mod 18]; H4 ≥ 0 → c3 × H4 at
/// F18[j mod 18], j from c2 when H3 ≥ 0, else 0 (Edge case 5).
fn meteor_hit(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    mm: &ClientMissile,
    [h1, h2, h3, h4]: [i32; 4],
    [c1, c2, c3]: [i32; 3],
) -> Result<(), HandlerError> {
    let t = env.skills.ok_or(HandlerError::Invalid(
        "missiles/client-bodies-2.md §B12 hit 18 needs the skills tables",
    ))?;
    let Some(skill) = usize::try_from(mm.skill).ok().and_then(|s| t.skills.get(s)) else {
        return Ok(());
    };
    let (x, y) = cell_of(mm);
    let mut rec = CreateRecord {
        owner: mm.owner,
        skill: mm.skill,
        level: mm.level,
        ..CreateRecord::default()
    };
    let at = |w: &mut ClientWorld, rec: &mut CreateRecord, dx: i32, dy: i32| {
        (rec.x, rec.y) = (x + dx, y + dy);
        create_from(w, env, m, rec)
    };
    if h1 >= 0 {
        (rec.flags, rec.class) = (flag::POSITION | flag::NO_LIGHT, h1 as u32);
        for i in 0..c1.max(0) as usize {
            at(w, &mut rec, M5_X[i % 5], M5_Y[i % 5])?;
        }
    }
    if h2 >= 0 {
        (rec.flags, rec.class) = (flag::POSITION | flag::RANGE, h2 as u32);
        rec.range = if mm.level >= 1 {
            (skill.param3 as i32).wrapping_add((mm.level - 1).wrapping_mul(skill.param4 as i32))
        } else {
            0
        };
        if let Some(c) = at(w, &mut rec, 0, 0)? {
            if let Some(id) = super::light_of(w, c) {
                w.lights.set_radius(id, METEOR_LIGHT);
            }
        }
    }
    rec.flags = flag::POSITION | flag::NO_LIGHT | flag::RANGE;
    if h3 >= 0 {
        rec.class = h3 as u32;
        for i in 0..c2.max(0) as usize {
            at(w, &mut rec, F18_X[i % 18], F18_Y[i % 18])?;
        }
    }
    if h4 >= 0 {
        rec.class = h4 as u32;
        let j0 = if h3 >= 0 { c2.max(0) } else { 0 };
        for j in j0..j0 + c3.max(0) {
            let j = j as usize;
            at(w, &mut rec, F18_X[j % 18], F18_Y[j % 18])?;
        }
    }
    Ok(())
}

/// Rocks with (m, n, v, g, f): O
/// none → nothing. g ≥ 0: n times tx := x + rnd(4n + 1) − 2n, ty likewise
/// (x first); the path distance from m to (tx, ty) > 3 → class 456 (flags 0x524: frames
/// from distance, arc, absolute target, velocity v; origin m). f ≥ 0: f
/// at (x, y) + F18[i], i = 0…17 (flags 1).
#[allow(clippy::too_many_arguments)]
fn rocks(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    mm: &ClientMissile,
    n: i32,
    v: i32,
    g: i32,
    f: i32,
) -> Result<(), HandlerError> {
    let Some(o) = mm.owner else {
        return Ok(());
    };
    let (x, y) = cell_of(mm);
    if g >= 0 {
        let mut rec = CreateRecord {
            flags: flag::VELOCITY | flag::TARGET_ABSOLUTE | flag::ARC | flag::FRAMES_FROM_DISTANCE,
            owner: Some(o),
            origin: Some(m),
            class: FLYING_ROCKS,
            velocity: v,
            skill: mm.skill,
            level: mm.level,
            ..CreateRecord::default()
        };
        for _ in 0..n.max(0) {
            let tx = x + rnd(w, m, 4 * n + 1) - 2 * n;
            let ty = y + rnd(w, m, 4 * n + 1) - 2 * n;
            let (dx, dy) = ((tx - x).abs(), (ty - y).abs());
            if dx.max(dy) + dx.min(dy) / 2 > 3 {
                (rec.tx, rec.ty) = (tx, ty);
                create_from(w, env, m, &rec)?;
            }
        }
    }
    if f >= 0 {
        let mut rec = CreateRecord {
            flags: flag::POSITION,
            owner: Some(o),
            class: f as u32,
            skill: mm.skill,
            level: mm.level,
            ..CreateRecord::default()
        };
        for i in 0..18 {
            (rec.x, rec.y) = (x + F18_X[i], y + F18_Y[i]);
            create_from(w, env, m, &rec)?;
        }
    }
    Ok(())
}

/// The client unit search with (C, x, y, r, mask, cb, ctx, skip)
/// up to the callback: C's room required (C a
/// model unit in a room list); x = 0 or y = 0 → pos(C); mask 0 → 0x583.
/// The rooms of C's room's adjacency array in order (mask 0x2000: town
/// rooms skipped; the near-box test never rejects, Edge case 1), each
/// room's units in list order, C skipped: V accepted when its squared
/// distance ≤ r², the filter passes and not (skip, V a monster, monstats
/// flag 27). Returns the accepted units in order.
fn search(
    w: &ClientWorld,
    env: &Env,
    c: UnitKey,
    (mut x, mut y): (i32, i32),
    r: i32,
    mask: u32,
    skip: bool,
) -> Result<Vec<UnitKey>, HandlerError> {
    let (Some(d), Some(room)) = (w.drlg.as_ref(), w.room_units.room_of(c)) else {
        return Ok(Vec::new());
    };
    let Some(cu) = w.units.get(&c) else {
        return Ok(Vec::new());
    };
    let mask = if mask == 0 { 0x583 } else { mask };
    if x == 0 || y == 0 {
        let (cx, cy) = cu.cell();
        (x, y) = (i32::from(cx), i32::from(cy));
    }
    let Some(adjacency) = d.drlg.active_room(room).map(|a| a.adjacency.clone()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for dr in adjacency {
        if mask & 0x2000 != 0 && room_in_town(w, dr) {
            continue;
        }
        for &v in w.room_units.list(dr) {
            if v == c {
                continue;
            }
            let Some(vu) = w.units.get(&v) else {
                continue;
            };
            let (vx, vy) = vu.cell();
            let (dx, dy) = (i64::from(x - i32::from(vx)), i64::from(y - i32::from(vy)));
            if dx * dx + dy * dy > i64::from(r) * i64::from(r) {
                continue;
            }
            if !filter(w, env, c, v, mask)? {
                continue;
            }
            if skip && v.unit_type == MONSTER && no_aura(w, env, v) {
                continue;
            }
            out.push(v);
        }
    }
    Ok(out)
}

/// Fire patch with (m, list, n, k, v, chance): m's room
/// required. For j := −k … k (x, outer), i := −k … k (y, inner), P :=
/// (x + j, y + i) is kept when i² + j² ≤ k², the line walk from P to
/// (x + 2j, y + 2i) (mask 4, from m's room; Edge case 6) is clear and the
/// room holding P (looked up from m's room) exists outside town. Per kept
/// point: v ≠ 0 → seed step, frames := v + (lo' mod 25) − 12; class :=
/// list[roll(n)]; seed step, lo' mod 100 < chance → create at P (flags 1,
/// 0x8001 with frames; owner O, m's skill and level). With no client
/// DRLG every point counts as in a room with a clear line (the
/// create's).
fn fire_patch(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    list: &[i32],
    k: i32,
    v: i32,
    chance: i32,
) -> Result<(), HandlerError> {
    let Some(mm) = w.objclient.missiles.get(&m).copied() else {
        return Ok(());
    };
    let (x, y) = cell_of(&mm);
    let room = match w.active_rooms {
        None => None,
        Some(_) => {
            let (Ok(cx), Ok(cy)) = (u16::try_from(x), u16::try_from(y)) else {
                return Ok(());
            };
            match w.room_at(cx, cy) {
                Some(r) => Some(r),
                None => return Ok(()),
            }
        }
    };
    for j in -k..=k {
        for i in -k..=k {
            let (px, py) = (x + j, y + i);
            if i * i + j * j > k * k
                || !patch_point(w, room.as_ref(), (px, py), (x + 2 * j, y + 2 * i))
            {
                continue;
            }
            let mut rec = CreateRecord {
                flags: flag::POSITION,
                owner: mm.owner,
                x: px,
                y: py,
                skill: mm.skill,
                level: mm.level,
                ..CreateRecord::default()
            };
            if v != 0 {
                rec.flags |= flag::RANGE;
                rec.range = v + (seed_step(w, m) % 25) as i32 - 12;
            }
            rec.class = list[rnd(w, m, list.len() as i32) as usize] as u32;
            if ((seed_step(w, m) % 100) as i32) < chance {
                create_from(w, env, m, &rec)?;
            }
        }
    }
    Ok(())
}

/// A fire patch point P: the line walk with (m's room, P, to, 4)
/// is clear and the room holding P (from m's room) is not a
/// town room. No client DRLG (`room` none) → kept.
fn patch_point(
    w: &ClientWorld,
    room: Option<&super::super::world::ActiveRoom>,
    p: (i32, i32),
    to: (i32, i32),
) -> bool {
    let (Some(room), Some(d)) = (room, w.drlg.as_ref()) else {
        return true;
    };
    let pt = |(x, y): (i32, i32)| d2_sim::path::coords::Point::new(x, y);
    let start = d.drlg.active_room(room.room).map(|a| a.id);
    if d2_sim::path::line::line_test(
        &super::MissileRooms(&d.drlg, &w.objclient.unit_grids),
        start,
        pt(p),
        pt(to),
        4,
    )
    .blocked()
    {
        return false;
    }
    w.cell_lookup(room, p.0, p.1)
        .is_some_and(|r| !d2_sim::drlg::is_town(u32::from(r.level)))
}

/// Hit 9's ally clause: the pet test or the owner test with (O, U).
/// Both need the pet-list owner of U's GUID
/// (the first record whose pet is that GUID; none → −1):
/// holds only when it is O's GUID, and the owner test fails for a monster
/// U without one. So a monster U outside the pet list is no ally target;
/// the other cases read O's left / right skill `TargetPet` /
/// `TargetAlly` and the party test, which the model does
/// not hold: handler errors.
fn ally_target(w: &ClientWorld, u: UnitKey) -> Result<bool, HandlerError> {
    let owner = w
        .pets
        .iter()
        .find(|r| r.pet == u.guid)
        .map_or(u32::MAX, |r| r.owner);
    if u.unit_type == MONSTER && owner == u32::MAX {
        return Ok(false);
    }
    Err(HandlerError::Invalid(
        "missiles/client-bodies-2.md §B12 hit 9: the ally target tests",
    ))
}

/// `monstats` flag 27 of V's class: `noaura` (flags +12 bit 27).
fn no_aura(w: &ClientWorld, env: &Env, v: UnitKey) -> bool {
    w.units
        .get(&v)
        .and_then(|u| env.monsters.get(u.class as usize))
        .and_then(|c| c.as_ref())
        .is_some_and(|c| c.no_aura)
}

/// Whether a client DRLG room is a town room.
fn room_in_town(w: &ClientWorld, room: d2_sim::drlg::DrlgRoomId) -> bool {
    w.active_rooms
        .as_deref()
        .and_then(|rooms| rooms.iter().find(|r| r.room == room))
        .is_some_and(|r| d2_sim::drlg::is_town(u32::from(r.level)))
}

/// The search filter with (C, V, mask); the tests the model does not hold (undead 0x4, boss 0x4000,
/// prime evil 0x40000, missiles 0x8, 0x10000, 0x20000) are handler
/// errors when a unit reaches them.
fn filter(
    w: &ClientWorld,
    env: &Env,
    c: UnitKey,
    v: UnitKey,
    mask: u32,
) -> Result<bool, HandlerError> {
    let Some(vu) = w.units.get(&v) else {
        return Ok(false);
    };
    let mode = vu.mode;
    let held = |what: &'static str| Err(HandlerError::Invalid(what));
    let pass = match v.unit_type {
        PLAYER => {
            mask & 0x1 != 0
                && if mask & 0x1000 != 0 {
                    mode == 17
                } else {
                    mode != 0 && mode != 17
                }
        }
        MONSTER => {
            if mask & 0x2 == 0
                || !(if mask & 0x1000 != 0 {
                    mode == 12
                } else {
                    mode != 0 && mode != 12
                })
            {
                false
            } else if mask & 0x4 != 0 {
                return held("missiles/client-bodies-2.md §B9: the undead test");
            } else if mask & 0x4000 != 0 {
                return held(
                    "missiles/client-bodies-2.md §B9: monster filter mask 0x4000 is not modelled",
                );
            } else if mask & 0x40000 != 0 {
                return held(
                    "missiles/client-bodies-2.md §B9: monster filter mask 0x40000 is not modelled",
                );
            } else {
                true
            }
        }
        OBJECT => mask & 0x10 != 0,
        MISSILE => {
            if mask & 0x8 != 0 {
                return held("missiles/client-bodies-2.md §B9: the missile test");
            }
            false
        }
        ITEM => mask & 0x20 != 0,
        _ => false,
    };
    if !pass {
        return Ok(false);
    }
    if mask & 0x10000 != 0 {
        return held("missiles/client-bodies-2.md §B9: target filter mask 0x10000 is not modelled");
    }
    if mask & 0x20000 != 0 && !in_melee_range(w, env, c, v, 0)? {
        return Ok(false);
    }
    let flags_ok = super::flags_4_8(w, v);
    Ok(!(mask & 0x80 != 0 && !flags_ok)
        && !(mask & 0x400 != 0 && !flags_ok)
        && !(mask & 0x100 != 0 && super::super::modes::in_town(w, v))
        && !(mask & 0x8000 != 0 && !super::super::combat::hostile_between(w, env.monsters, c, v))
        && !(mask & 0x80000 != 0 && super::just_hit(w, v))
        && !(mask & 0x200 != 0 && !line_clear(w, c, v)))
}

/// Mask 0x20000 and the client do's reach test: in melee range
/// with (a, b, extra) over the client
/// units, through [`d2_sim::combat::range::in_melee_range`]. Distance
/// from the cells and sizes (a player 2, a monster its
/// `monstats2` size); the line with (a, b, 0x804) on the client
/// collision rooms. The melee range of a player is the
/// `rangeadder` of its weapon on body location 4, else 5,
/// read for the local player's items; a remote player or a monster as
/// `a` is a handler error (no caller here).
pub(crate) fn in_melee_range(
    w: &ClientWorld,
    env: &Env,
    a: UnitKey,
    b: UnitKey,
    extra: i32,
) -> Result<bool, HandlerError> {
    let reach = melee_reach(w, env, a)?;
    let range = ClientRange { w, env, reach };
    Ok(d2_sim::combat::range::in_melee_range(
        &range,
        Some(a),
        Some(b),
        extra,
    ))
}

/// The melee range with (a) of the local
/// player: the `rangeadder` of its weapon on body location 4, else 5;
/// 0 without one. A monster: 0 here, its range being the
/// range world's `MeleeRng` (`melee_rng`).
/// Another unit is a handler error.
pub(crate) fn melee_reach(w: &ClientWorld, _env: &Env, a: UnitKey) -> Result<i32, HandlerError> {
    if a.unit_type == MONSTER {
        // A monster's range is the range world's `MeleeRng` (`melee_rng`).
        return Ok(0);
    }
    if a.unit_type != PLAYER || w.local_player != Some(a) {
        return Err(HandlerError::Invalid(
            "combat/hit.md §7.3: the melee range of a unit other than the local player",
        ));
    }
    let tables = w.item_tables.0.clone().ok_or(HandlerError::Invalid(
        "combat/hit.md §7.3: the melee range needs the item tables",
    ))?;
    Ok([4u8, 5]
        .into_iter()
        .find_map(|loc| {
            super::super::items::local_items(w)
                .into_iter()
                .find(|i| i.mode == super::super::items::mode::BODY && i.body == loc)
                .and_then(|i| i.code)
                .and_then(|c| tables.weapon_range_adder(c))
        })
        .unwrap_or(0))
}

/// The [`d2_sim::combat::range::RangeWorld`] of [`in_melee_range`]: only
/// the queries its rules read for a player `a` are answered.
struct ClientRange<'a, 'e> {
    w: &'a ClientWorld,
    env: &'a Env<'e>,
    reach: i32,
}

impl ClientRange<'_, '_> {
    fn size(&self, u: UnitKey) -> i32 {
        match u.unit_type {
            PLAYER => 2,
            MONSTER => self
                .w
                .units
                .get(&u)
                .and_then(|c| self.env.monsters.get(c.class as usize))
                .and_then(|c| c.as_ref())
                .map_or(0, |c| i32::from(c.size_x)),
            _ => 0,
        }
    }
}

impl d2_sim::combat::range::RangeWorld for ClientRange<'_, '_> {
    type Unit = UnitKey;
    fn unit_type(&self, u: UnitKey) -> d2_sim::units::UnitType {
        match u.unit_type {
            PLAYER => d2_sim::units::UnitType::Player,
            MONSTER => d2_sim::units::UnitType::Monster,
            OBJECT => d2_sim::units::UnitType::Object,
            MISSILE => d2_sim::units::UnitType::Missile,
            ITEM => d2_sim::units::UnitType::Item,
            _ => d2_sim::units::UnitType::Tile,
        }
    }
    fn owner(&self, _: UnitKey) -> Option<UnitKey> {
        None
    }
    fn missile_source(&self, _: UnitKey) -> Option<UnitKey> {
        None
    }
    fn guid(&self, u: UnitKey) -> u32 {
        u.guid
    }
    fn relation_flags(&self, _: UnitKey, _: u32) -> Option<u32> {
        None
    }
    fn alignment_stat(&self, _: UnitKey) -> i32 {
        0
    }
    fn base_id(&self, u: UnitKey) -> i32 {
        self.w
            .units
            .get(&u)
            .and_then(|c| self.env.monsters.get(c.class as usize))
            .and_then(|c| c.as_ref())
            .map_or(-1, |c| i32::from(c.base_id))
    }
    fn distance(&self, a: UnitKey, b: UnitKey) -> i32 {
        let (Some(au), Some(bu)) = (self.w.units.get(&a), self.w.units.get(&b)) else {
            return i32::MAX;
        };
        super::super::objects::unit_distance_at(au.cell(), self.size(a), bu.cell(), self.size(b))
    }
    fn line_blocked(&self, a: UnitKey, b: UnitKey, mask: u32) -> bool {
        let Some(d) = self.w.drlg.as_ref() else {
            return false;
        };
        let end = |u: UnitKey| {
            let (x, y) = self.w.units.get(&u).map_or((0, 0), |c| c.cell());
            d2_sim::path::line::LineUnit {
                room: self
                    .w
                    .room_units
                    .room_of(u)
                    .and_then(|r| d.drlg.active_room(r).map(|a| a.id)),
                x: i32::from(x),
                y: i32::from(y),
                size: self.size(u),
            }
        };
        d2_sim::path::line::units_line_blocked(
            &super::MissileRooms(&d.drlg, &self.w.objclient.unit_grids),
            &end(a),
            &end(b),
            mask as u16,
        )
    }
    fn weapon_range_adder(&self, _: UnitKey) -> i32 {
        self.reach
    }
    fn melee_rng(&self, u: UnitKey) -> Option<u8> {
        self.w
            .units
            .get(&u)
            .and_then(|c| self.env.monsters.get(c.class as usize))
            .and_then(|c| c.as_ref())
            .map(|c| c.path_byte)
    }
    fn weapon_class_now(&self, _: UnitKey) -> i32 {
        0
    }
}

/// Mask 0x200: C and V have rooms and the line walk with (C's
/// room, pos(C), pos(V), 4) = 0.
fn line_clear(w: &ClientWorld, c: UnitKey, v: UnitKey) -> bool {
    let (Some(d), Some(cr), Some(_)) = (
        w.drlg.as_ref(),
        w.room_units.room_of(c),
        w.room_units.room_of(v),
    ) else {
        return false;
    };
    let (Some(cu), Some(vu)) = (w.units.get(&c), w.units.get(&v)) else {
        return false;
    };
    let p = |u: &super::super::world::ClientUnit| {
        let (x, y) = u.cell();
        d2_sim::path::coords::Point::new(i32::from(x), i32::from(y))
    };
    let room = d.drlg.active_room(cr).map(|a| a.id);
    !d2_sim::path::line::line_test(
        &super::MissileRooms(&d.drlg, &w.objclient.unit_grids),
        room,
        p(cu),
        p(vu),
        4,
    )
    .blocked()
}

/// The next-GUID pick with (C, x, y, r, mask, g): mask :=
/// (mask or 0x583) | 0xA280; among the accepted units the smallest GUID
/// above g, else the smallest at or below g (ties: the later unit).
pub(super) fn next_guid(
    w: &ClientWorld,
    env: &Env,
    c: UnitKey,
    at: (i32, i32),
    r: i32,
    mask: u32,
    g: u32,
) -> Result<Option<UnitKey>, HandlerError> {
    Ok(pick(w, env, c, at, r, mask, g)?.0)
}

/// [`next_guid`] with its count out (`&n`): the callback
/// adds 1 each time it replaces the best (GUID above g, below the best's)
/// or the fallback (GUID at or below the fallback's, which starts at g).
pub(crate) fn pick(
    w: &ClientWorld,
    env: &Env,
    c: UnitKey,
    at: (i32, i32),
    r: i32,
    mask: u32,
    g: u32,
) -> Result<(Option<UnitKey>, i32), HandlerError> {
    let mask = (if mask == 0 { 0x583 } else { mask }) | 0xA280;
    let mut best: Option<UnitKey> = None;
    let mut fallback: Option<UnitKey> = None;
    let mut n = 0;
    for v in search(w, env, c, at, r, mask, false)? {
        if v.guid > g {
            if best.is_none_or(|b| v.guid < b.guid) {
                best = Some(v);
                n += 1;
            }
        } else if fallback.is_none_or(|f| v.guid <= f.guid) {
            fallback = Some(v);
            n += 1;
        }
    }
    Ok((best.or(fallback), n))
}

/// The count with (C, x, y, r, mask): the core with mask |
/// 0xA783, skipflag 0, callback (each accepted unit adds 1).
pub(crate) fn count(
    w: &ClientWorld,
    env: &Env,
    c: UnitKey,
    at: (i32, i32),
    r: i32,
    mask: u32,
) -> Result<i32, HandlerError> {
    Ok(search(w, env, c, at, r, mask | 0xA783, false)?.len() as i32)
}

/// Bolt to each: up to `max` (none:
/// no cap) accepted units, in search order, get a bolt of `class` (flags
/// 0x20, owner O, origin m, target pos(V), m's skill and level).
#[allow(clippy::too_many_arguments)]
fn bolt_to_each(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    o: UnitKey,
    targets: Vec<UnitKey>,
    max: Option<i32>,
    class: i32,
) -> Result<(), HandlerError> {
    let Some(mm) = w.objclient.missiles.get(&m).copied() else {
        return Ok(());
    };
    let mut n = 0;
    for v in targets {
        if max.is_some_and(|max| n >= max) {
            break;
        }
        let Some((tx, ty)) = w.units.get(&v).map(|u| u.cell()) else {
            continue;
        };
        let rec = CreateRecord {
            flags: flag::TARGET_ABSOLUTE,
            owner: Some(o),
            origin: Some(m),
            class: class as u32,
            tx: i32::from(tx),
            ty: i32::from(ty),
            skill: mm.skill,
            level: mm.level,
            ..CreateRecord::default()
        };
        create_from(w, env, m, &rec)?;
        n += 1;
    }
    Ok(())
}

/// The skills row of m's skill and the skill evaluator on it, for the hit
/// bodies.
fn skill_of<'a>(
    w: &ClientWorld,
    env: &Env<'a>,
    mm: &ClientMissile,
    what: &'static str,
) -> Result<Option<&'a d2_data::tables::Skills>, HandlerError> {
    let t = env.skills.ok_or(HandlerError::Invalid(what))?;
    let _ = w;
    Ok(usize::try_from(mm.skill).ok().and_then(|s| t.skills.get(s)))
}

fn eval(w: &ClientWorld, env: &Env, mm: &ClientMissile, m: UnitKey, field: u32) -> i32 {
    let Some(t) = env.skills else {
        return 0;
    };
    let mut units = super::super::passive::ClientSkills::new(w, mm.owner.unwrap_or(m));
    d2_sim::skills::levels::eval_skill(&mut units, t, mm.owner, field, mm.skill, mm.level)
}

/// The radius of hits 16, 25 and 26: c1 > 0 ? c1: max(eval(
/// `aurarangecalc`), 1).
fn aura_range(
    w: &ClientWorld,
    env: &Env,
    mm: &ClientMissile,
    m: UnitKey,
    c1: i32,
    skill: &d2_data::tables::Skills,
) -> i32 {
    if c1 > 0 {
        c1
    } else {
        eval(w, env, mm, m, skill.aurarangecalc).max(1)
    }
}

/// Hit 16 (chain lightning): O and
/// U required (the overlay on U: no model overlays). n := d28, ≤ 1 →
/// done; r := the aura range (no skills row → done); X := the next-GUID
/// pick (O, (x, y), r, 0x88583, U's GUID); X ≠ U → m's class created
/// (flags 0x21: start (x, y), target pos(X), target unit X), its d28 := n
/// − 1 and last-collided := U.
fn chain(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    mm: &ClientMissile,
    u: Option<UnitKey>,
    c1: i32,
) -> Result<(), HandlerError> {
    let (Some(o), Some(u)) = (mm.owner, u) else {
        return Ok(());
    };
    let n = mm.d28;
    if n <= 1 {
        return Ok(());
    }
    let Some(skill) = skill_of(
        w,
        env,
        mm,
        "missiles/client-bodies-2.md §B12 hit 16 needs the skills tables",
    )?
    else {
        return Ok(());
    };
    let r = aura_range(w, env, mm, m, c1, skill);
    let at = cell_of(mm);
    let Some(x) = next_guid(w, env, o, at, r, 0x88583, u.guid)? else {
        return Ok(());
    };
    if x == u {
        return Ok(());
    }
    let class = w.objclient.set_c.get(&m).map_or(0, |c| c.class);
    let Some((tx, ty)) = w.units.get(&x).map(|v| v.cell()) else {
        return Ok(());
    };
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE,
        owner: Some(o),
        target: Some(x),
        class,
        x: at.0,
        y: at.1,
        tx: i32::from(tx),
        ty: i32::from(ty),
        skill: mm.skill,
        level: mm.level,
        ..CreateRecord::default()
    };
    if let Some(c) = create_from(w, env, m, &rec)? {
        if let Some(cm) = w.objclient.missiles.get_mut(&c) {
            cm.d28 = n - 1;
            cm.last_collided = Some(u);
        }
    }
    Ok(())
}

/// Hit 25 (lightning fury): H1 ≥
/// 0, O and the skills row required. r := the aura range; max := c2 > 0 ?
/// c2: max(eval(`calc1`), 1); mask := `aurafilter`, 0 → 0xA783; the
/// search (O, (x, y), r, mask, skip 1) with bolt to each H1.
fn fury(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    mm: &ClientMissile,
    h1: i32,
    [c1, c2, _]: [i32; 3],
) -> Result<(), HandlerError> {
    let (true, Some(o)) = (h1 >= 0, mm.owner) else {
        return Ok(());
    };
    let Some(skill) = skill_of(
        w,
        env,
        mm,
        "missiles/client-bodies-2.md §B12 hit 25 needs the skills tables",
    )?
    else {
        return Ok(());
    };
    let r = aura_range(w, env, mm, m, c1, skill);
    let max = if c2 > 0 {
        c2
    } else {
        eval(w, env, mm, m, skill.calc1).max(1)
    };
    let mask = if skill.aurafilter != 0 {
        skill.aurafilter
    } else {
        0xA783
    };
    let targets = search(w, env, o, cell_of(mm), r, mask, true)?;
    bolt_to_each(w, env, m, o, targets, Some(max), h1)
}

/// Hit 26 (fist of the heavens):
/// the server `HitSubMissile1` ≥ 0, O and the skills row required; V :=
/// the unit (type d28, GUID d2C), none → 0. r := the aura range; mask :=
/// `aurafilter` ≠ 0 ? 0xA683 : 0 (0x583); the search (O, (x, y), r, mask,
/// skip 0) with bolt to each H1 and no cap (PROVISIONAL, edge case 2:
/// no cap; settled by counting the client bolts of a cast over a Fallen
/// pack).
fn fist(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    mm: &ClientMissile,
    row: &super::ClientMissileRow,
    h1: i32,
    c1: i32,
) -> Result<bool, HandlerError> {
    let (true, Some(o)) = (row.hit_sub1_server >= 0, mm.owner) else {
        return Ok(true);
    };
    let Some(skill) = skill_of(
        w,
        env,
        mm,
        "missiles/client-bodies-2.md §B12 hit 26 needs the skills tables",
    )?
    else {
        return Ok(true);
    };
    let v = u8::try_from(mm.d28)
        .ok()
        .map(|t| UnitKey::new(t, mm.d2c as u32))
        .filter(|v| w.units.contains_key(v) || w.objclient.set_c.contains_key(v));
    if v.is_none() {
        return Ok(false);
    }
    let r = aura_range(w, env, mm, m, c1, skill);
    let mask = if skill.aurafilter != 0 { 0xA683 } else { 0 };
    let targets = search(w, env, o, cell_of(mm), r, mask, false)?;
    bolt_to_each(w, env, m, o, targets, None, h1)?;
    Ok(true)
}

/// Hit 13 (guided arrow / bone
/// spirit): O none or dead, or m's room in town → 1. f := d28; U none and
/// f & 4 → 1. f & 1: U none → 1, U not the path target unit → 0, else 1.
/// f & 2: frames left > 0 → (f & 4 ? 1: 0), else the retarget's result
/// ≠ 0. Else 1.
fn guided_hit(
    w: &mut ClientWorld,
    env: &Env,
    m: UnitKey,
    mm: &ClientMissile,
    u: Option<UnitKey>,
) -> Result<bool, HandlerError> {
    let Some(o) = mm
        .owner
        .filter(|o| w.units.get(o).is_some_and(|x| !x.is_dead()))
    else {
        return Ok(true);
    };
    if super::in_town(w, m) {
        return Ok(true);
    }
    let f = mm.d28;
    if u.is_none() && f & 4 != 0 {
        return Ok(true);
    }
    if f & 1 != 0 {
        return Ok(match u {
            None => true,
            Some(u) => mm.target_unit == Some(u),
        });
    }
    if f & 2 != 0 {
        if mm.current > 0 {
            return Ok(f & 4 != 0);
        }
        return retarget(w, env, m, o);
    }
    Ok(true)
}

/// Retarget: 1 with no row or
/// d28 & 4; else T := the next-GUID pick (O, (x, y), `Param2` (server),
/// mask 3, 0xFFFFFFFF), then the retarget of m to T, and 0. The retarget: T
/// dead → none; total := left := `Range` + `LevRange` × (level − 1); T
/// given: path target unit T, d28 := 5, re-path when with (m, T)
/// < 25; else target point (x + a, y + b) with a, b the i16 halves of
/// d2C, d28 := 6, re-path when with (m, x + a, y + b) < 25.
fn retarget(w: &mut ClientWorld, env: &Env, m: UnitKey, o: UnitKey) -> Result<bool, HandlerError> {
    let rate = w.tick_rate;
    let Some(mm) = w.objclient.missiles.get(&m).copied() else {
        return Ok(true);
    };
    let class = w.objclient.set_c.get(&m).map_or(0, |c| c.class);
    let Some(row) = env.rows.get(class as usize).copied() else {
        return Ok(true);
    };
    if mm.d28 & 4 != 0 {
        return Ok(true);
    }
    let at = cell_of(&mm);
    let t = next_guid(w, env, o, at, row.param[1], 3, u32::MAX)?
        .filter(|t| !w.units.get(t).is_some_and(|x| x.is_dead()));
    let frames = i32::from(row.range) + i32::from(row.lev_range) * (mm.level - 1);
    let t_at = t.and_then(|t| w.units.get(&t).map(|x| x.cell()));
    let near = match (t, t_at) {
        (Some(t), Some(c)) => Some(super::super::objects::distance_at(
            (at.0 as u16, at.1 as u16),
            super::unit_size(env, w, m),
            c,
            super::unit_size(env, w, t),
        )),
        _ => None,
    };
    let Some(mm) = w.objclient.missiles.get_mut(&m) else {
        return Ok(false);
    };
    (mm.total, mm.current) = (rate.ticks(frames), rate.ticks(frames));
    match (t, t_at) {
        (Some(t), Some(c)) => {
            mm.target_unit = Some(t);
            mm.d28 = 5;
            if near.is_some_and(|d| d < 25) {
                super::repath_to(mm, (i32::from(c.0), i32::from(c.1)));
            }
        }
        _ => {
            let a = i32::from(mm.d2c as i16);
            let b = i32::from((mm.d2c >> 16) as i16);
            let to = (at.0 + a, at.1 + b);
            mm.target_point = to;
            mm.target_unit = None;
            mm.d28 = 6;
            let (dx, dy) = (a.abs(), b.abs());
            if dx.max(dy) + dx.min(dy) / 2 < 25 {
                super::repath(mm);
            }
        }
    }
    Ok(false)
}
