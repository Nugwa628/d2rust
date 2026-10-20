//! The client do functions of the monster skills 46–54 (`cltdofunc`,
//! table), reached from [`super::client_do::client_do`]:
//! the sequence steps of the maggots, Swarm and the frog demons (46, 47,
//! 49, 52), Andariel's spray (48), the crow nest's lay (50), the
//! gargoyle trap's shot (51) and the fetish shaman's aura disc and
//! inferno flames (53, 54).
//!
//! Function 50 (Nest, EvilHutSpawner) clears
//! collision bit 0x100 at the point held in the used skill entry's
//! params 2 and 3; they read 0 here, so it returns 0
//! ([`super::client_do::client_do`]'s arm).
//!
//! Not modelled (each named where it would run): the used skill entry's
//! params and flags (the
//! model holds no client skill entry, so they read 0) and the flame's
//! path max distance.

use super::client_do::{
    calc1, cast_target_point, cltcalc1, create_toward, owner_dir64, path_target_unit, set_flag_40,
    skill_calc, unit_roll, valid_missile,
};
use super::client_missiles::{create, flag, CreateRecord};
use super::dispatch::HandlerError;
use super::world::{ClientWorld, ModelInputs, UnitKey, MONSTER};
use d2_sim::skills::use_::bodies::b4_helpers::{dir8, pair};

/// The spray's first offset by 8-direction (bytes).
const SPRAY_FIRST: [u8; 8] = [29, 28, 27, 26, 25, 24, 31, 30];

/// The spray's second offset by 8-direction and frame (bytes,
/// 8 rows of 9); 99: none.
const SPRAY_SECOND: [[u8; 9]; 8] = [
    [27, 14, 15, 3, 99, 7, 21, 22, 31],
    [26, 12, 13, 2, 99, 6, 19, 20, 30],
    [25, 10, 11, 1, 99, 5, 17, 18, 29],
    [24, 8, 9, 0, 99, 4, 15, 16, 28],
    [31, 22, 23, 7, 99, 3, 13, 14, 27],
    [30, 20, 7, 6, 99, 2, 1, 12, 26],
    [29, 18, 19, 5, 99, 1, 9, 10, 25],
    [28, 16, 17, 4, 99, 0, 23, 8, 24],
];

/// The gargoyle trap's quadrant by 64-direction (bytes).
const TRAP_QUADRANT: [u8; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0,
];

/// The gargoyle trap's path direction by quadrant (bytes).
const TRAP_DIRECTION: [u8; 4] = [49, 31, 17, 0];

/// The flame velocity jitter.
const FLAME_JITTER: [i32; 5] = [-2, -1, 0, 1, 2];

/// DiabLight's velocity jitter.
const DIAB_LIGHT_JITTER: [i32; 5] = [-1, -3, -2, -1, 0];

/// U has a used skill entry: a monster's used skill, a
/// player's current skill.
fn has_entry(w: &ClientWorld, key: UnitKey) -> bool {
    w.units.get(&key).is_some_and(|u| {
        if key.unit_type == MONSTER {
            u.used_skill.is_some()
        } else {
            u.skills.as_ref().is_some_and(|l| l.current.is_some())
        }
    })
}

/// The 64-direction from U's cell to the centre of (x, y).
fn dir64_to(w: &ClientWorld, key: UnitKey, (x, y): (i32, i32)) -> Option<u8> {
    let t = super::predict::path_tables()?;
    let (ux, uy) = w.units.get(&key)?.cell();
    let centre = |c: i32| ((c as u32) << 16) | 0x8000;
    let (_, d) = d2_sim::path::walk::geom::direction_vector(
        t,
        (centre(i32::from(ux)), centre(i32::from(uy))),
        (centre(x), centre(y)),
    );
    Some(d & 63)
}

/// `cltdofunc` 46 (MaggotEgg): the sequence step
/// (+0x3C) := 0; 1.
pub(super) fn maggot_egg(w: &mut ClientWorld, key: UnitKey) -> i32 {
    if let Some(q) = w.units.get_mut(&key).and_then(|u| u.sequence.as_mut()) {
        q.step = 0;
    }
    1
}

/// `cltdofunc` 47 and 52 (MagottDown, Submerge):
/// the event byte (+0x4E) := 0;
/// the frame (+0x44 >> 8) below 1 → the sequence step := 0; 1.
pub(super) fn submerge_frame(w: &mut ClientWorld, key: UnitKey) -> i32 {
    if let Some(u) = w.units.get_mut(&key) {
        let cur = u.frame >> 8;
        u.action_frame = 0;
        if cur < 1 {
            if let Some(q) = u.sequence.as_mut() {
                q.step = 0;
            }
        }
    }
    1
}

/// `cltdofunc` 48 (AndrialSpray): m :=
/// `cltmissilea`, invalid → 0; no entry → 0; the aim T :=
/// [`cast_target_point`] (the entry's params read 0); b := the
/// 8-direction of the direction to T; P := U's cell + the offset of
/// [`SPRAY_FIRST`] b, + the offset of [`SPRAY_SECOND`] [b][i] (i := the
/// frame − 4 in 0…8; 99 none); one create toward P; 1.
pub(super) fn andariel_spray(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) || !has_entry(w, key) {
        return Ok(0);
    }
    let t = cast_target_point(w, key);
    let b = dir8(i32::from(dir64_to(w, key, t).unwrap_or(0))) as usize;
    let Some(u) = w.units.get(&key) else {
        return Ok(0);
    };
    let (ux, uy) = u.cell();
    let (dx, dy) = pair(usize::from(SPRAY_FIRST[b]));
    let (mut px, mut py) = (i32::from(ux) + dx, i32::from(uy) + dy);
    let i = ((u.frame >> 8) - 4).clamp(0, 8) as usize;
    let e = SPRAY_SECOND[b][i];
    if e != 99 {
        let (dx, dy) = pair(usize::from(e));
        px += dx;
        py += dy;
    }
    create_toward(w, inputs, key, m, (px, py), skill, level)?;
    Ok(1)
}

/// `cltdofunc` 49 (Swarm Move): no entry → 0;
/// v := the `calc1` value (the entry's flag 2, which takes `calc2`, reads
/// 0); the sequence position (+0x38) := (position & 0xFF) + v · 256;
/// 1.
pub(super) fn swarm_move(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    if !has_entry(w, key) {
        return Ok(0);
    }
    let v = calc1(w, inputs, key, skill, level)?;
    if let Some(q) = w.units.get_mut(&key).and_then(|u| u.sequence.as_mut()) {
        q.pos = (q.pos & 0xFF).wrapping_add(v.wrapping_mul(256));
    }
    Ok(1)
}

/// `cltdofunc` 51 (GargoyleTrap): m :=
/// `cltmissilea`, invalid → 0; flag 0x40 on U; T := U's path target unit,
/// none → 0. P := (U.x stepped 4 times toward T.x, T.y) when |T.x − U.x| <
/// |T.y − U.y|, else (T.x, U.y stepped 4 times toward T.y). U's path
/// direction := [`TRAP_DIRECTION`] [[`TRAP_QUADRANT`] [dir64(U, P)] & 3];
/// one create, flags 3, owner U, start (P − U) / 6 + U −
/// 1 per axis (truncating), relative target P − U; 1.
pub(super) fn gargoyle_trap(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    set_flag_40(w, key);
    let Some(t) = path_target_unit(w, key) else {
        return Ok(0);
    };
    let cell = |k: UnitKey| w.units.get(&k).map(|u| u.cell());
    let (Some((tx, ty)), Some((ux, uy))) = (cell(t), cell(key)) else {
        return Ok(0);
    };
    let (tx, ty, ux, uy) = (i32::from(tx), i32::from(ty), i32::from(ux), i32::from(uy));
    let toward = |mut a: i32, b: i32| {
        for _ in 0..4 {
            a += (b - a).signum();
        }
        a
    };
    let (px, py) = if (tx - ux).abs() < (ty - uy).abs() {
        (toward(ux, tx), ty)
    } else {
        (tx, toward(uy, ty))
    };
    let (rx, ry) = (px - ux, py - uy);
    let d = dir64_to(w, key, (px, py)).unwrap_or(0);
    let dir = TRAP_DIRECTION[usize::from(TRAP_QUADRANT[usize::from(d)] & 3)];
    super::critter_path::set_direction(w, super::critter_path::Who::server(key), dir);
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE,
        owner: Some(key),
        class: m as u32,
        x: rx / 6 + ux - 1,
        y: ry / 6 + uy - 1,
        tx: rx,
        ty: ry,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    Ok(1)
}

/// `cltdofunc` 53 (FetishAura): m :=
/// `cltmissilea`, invalid → 0; flag 0x40 on U; T := [`cast_target_point`];
/// r := the `aurarangecalc` value − 6, at least 1; c := the `cltcalc1`
/// value; the disc of m around T with radius r, chance c and
/// step 1 (rolls on U's seed); 1.
pub(super) fn fetish_aura(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    set_flag_40(w, key);
    let t = cast_target_point(w, key);
    let r = (skill_calc(w, inputs, key, skill, level, |r| r.aurarangecalc)? - 6).max(1);
    let c = skill_calc(w, inputs, key, skill, level, |r| r.cltcalc1)?;
    disc(w, inputs, key, t, r, m, skill, level, c)?;
    Ok(1)
}

/// The disc with (U, x, y, r, m, skill, level, chance, 1) of a
/// player or monster U (with U as its own owner): record flags 0x201, owner U, class m, skill, level; for i (y,
/// outer) and j (x, inner) from −r to r with i² + j² ≤ r²: chance > 0 and
/// rnd(chance) ≠ 0 skips the point; start (x + j, y + i); start frame :=
/// rnd(m's `RandStart`) (0 when that is ≤ 0); the room holding the start,
/// searched from U's room, none → skipped; else create. The rolls step
/// U's seed.
#[allow(clippy::too_many_arguments)]
fn disc(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    (x, y): (i32, i32),
    r: i32,
    m: i32,
    skill: u16,
    level: i32,
    chance: i32,
) -> Result<(), HandlerError> {
    let rand_start = usize::try_from(m)
        .ok()
        .and_then(|i| inputs.tables.missiles.get(i))
        .map_or(0, |row| row.rand_start);
    let from = w
        .units
        .get(&key)
        .map(|u| u.cell())
        .and_then(|(ux, uy)| w.room_at(ux, uy));
    for i in -r..=r {
        for j in -r..=r {
            if i * i + j * j > r * r {
                continue;
            }
            if chance > 0 && unit_roll(w, key, chance) != 0 {
                continue;
            }
            let (px, py) = (x + j, y + i);
            let start_frame = unit_roll(w, key, rand_start) as i32;
            let in_room = w.active_rooms.is_none()
                || (u16::try_from(px).ok())
                    .zip(u16::try_from(py).ok())
                    .is_some_and(|(px, py)| w.room_from(from.as_ref(), px, py).is_some());
            if !in_room {
                continue;
            }
            let rec = CreateRecord {
                flags: flag::POSITION | flag::START_FRAME,
                owner: Some(key),
                class: m as u32,
                x: px,
                y: py,
                skill: i32::from(skill),
                level,
                start_frame,
                owner_dir64: owner_dir64(w, key),
                ..CreateRecord::default()
            };
            create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
        }
    }
    Ok(())
}

/// `cltdofunc` 56 (DiabLight):
/// m := `cltmissilea`,
/// invalid → 0; E := the used entry, none → 0; v := the `cltcalc1` value;
/// n := E's param; n + 1 < v → E's param := n + 1, 0. Else E's param :=
/// 0; T := [`cast_target_point`], zero → 0; a monster U's path turns
/// toward T; one [`flame`] with [`DIAB_LIGHT_JITTER`];
/// the frame hold [`light_hold`]; 1.
pub(super) fn diablo_light(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let m = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, m) {
        return Ok(0);
    }
    let Some(n) = super::skills::used_entry(w, key).map(|e| e.param) else {
        return Ok(0);
    };
    let v = cltcalc1(w, inputs, key, skill, level)?;
    let set_param = |w: &mut ClientWorld, p: i32| {
        if let Some(e) = super::skills::used_entry_mut(w, key) {
            e.param = p;
        }
    };
    if n.wrapping_add(1) < v {
        set_param(w, n + 1);
        return Ok(0);
    }
    set_param(w, 0);
    let t = cast_target_point(w, key);
    if t.0 == 0 || t.1 == 0 {
        return Ok(0);
    }
    if key.unit_type == MONSTER {
        if let (Ok(x), Ok(y)) = (u16::try_from(t.0), u16::try_from(t.1)) {
            super::critter_path::face_toward(w, key, x, y);
        }
    }
    flame(w, inputs, key, m, t, skill, level, &DIAB_LIGHT_JITTER)?;
    light_hold(w, inputs, key, row.monanim);
    Ok(1)
}

/// The frame hold with (row, U): a monster U with a `monstats2`
/// row, b := its `InfernoAnim` (+0x109): the skill's `monanim` 14 (SQ) →
/// the sequence position := b − 1 and the frame count := (b + 1) · 256;
/// any other → the frame (+0x44) := (b − 1) · 256. A monster without a
/// row: nothing. Any other unit: the position := 8 and the count := 0x500.
fn light_hold(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, monanim: u8) {
    let b = if key.unit_type == MONSTER {
        let Some(b) = w
            .units
            .get(&key)
            .and_then(|u| inputs.tables.monsters.get(u.class as usize))
            .and_then(|r| r.as_ref())
            .map(|r| i32::from(r.inferno_anim))
        else {
            return;
        };
        Some(b)
    } else {
        None
    };
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    let set_pos = |u: &mut crate::bridge::world::ClientUnit, p: i32| {
        if let Some(q) = u.sequence.as_mut() {
            q.pos = (q.pos & 0xFF) + (p << 8);
        }
    };
    match b {
        Some(b) if monanim == 14 => {
            set_pos(u, b - 1);
            u.frame_count = (b + 1) << 8;
        }
        Some(b) => u.frame = (b << 8) - 0x100,
        None => {
            set_pos(u, 8);
            u.frame_count = 0x500;
        }
    }
}

/// `cltdofunc` 54 (FetishInferno, Baal Inferno,
/// MegademonInferno, Horror Arctic Blast): a :=
/// `cltmissilea`, invalid → 0; b := `cltmissileb`. T :=
/// [`cast_target_point`]; a monster U's path turns toward T.
/// Twice: the class a, or b when b > 0 and one roll of
/// U's seed is odd; then [`flame`]. 1.
pub(super) fn fetish_inferno(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let a = i32::from(row.cltmissile_abc[0]);
    if !valid_missile(inputs, a) {
        return Ok(0);
    }
    let b = i32::from(row.cltmissile_abc[1]);
    let t = cast_target_point(w, key);
    if key.unit_type == MONSTER {
        if let (Ok(x), Ok(y)) = (u16::try_from(t.0), u16::try_from(t.1)) {
            super::critter_path::face_toward(w, key, x, y);
        }
    }
    for _ in 0..2 {
        let m = if b > 0 && unit_roll(w, key, 2) != 0 {
            b
        } else {
            a
        };
        flame(w, inputs, key, m, t, skill, level, &FLAME_JITTER)?;
    }
    Ok(1)
}

/// The flame with (table, …): one roll of U's seed, ≠ 0 mod 4 →
/// record flags 0x4025 (no light), else 0x25; owner U, class m, start
/// U's cell, absolute target T, skill, level; m invalid → none. A second
/// roll: velocity := m's `Vel` + `jitter` [roll mod 5] (the caller's
/// table: [`FLAME_JITTER`], [`DIAB_LIGHT_JITTER`]). A made missile takes
/// its range ([`flame_range`]).
#[allow(clippy::too_many_arguments)]
fn flame(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    m: i32,
    (tx, ty): (i32, i32),
    skill: u16,
    level: i32,
    jitter: &[i32; 5],
) -> Result<(), HandlerError> {
    let mut flags = flag::POSITION | flag::VELOCITY | flag::TARGET_ABSOLUTE;
    if unit_roll(w, key, 4) != 0 {
        flags |= flag::NO_LIGHT;
    }
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(());
    };
    if !valid_missile(inputs, m) {
        return Ok(());
    }
    let jitter = jitter[unit_roll(w, key, 5) as usize];
    let rec = CreateRecord {
        flags,
        owner: Some(key),
        class: m as u32,
        x: i32::from(x),
        y: i32::from(y),
        tx,
        ty,
        velocity: inputs.tables.missiles[m as usize].vel + jitter,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    if let Some(k) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? {
        flame_range(w, inputs, key, k, m, skill, level)?;
    }
    Ok(())
}

/// With (skill, missile, U, level): n := the skill's `calc1`; n ≤
/// 0 → m's `Param2` (+0x3C, "minimum range") + level − 1; n clamped to
/// 1…255. The missile's total
/// and frames left (+0x0E, +0x10) := n, its
/// animation speed := (`AnimLen` (a byte) · 256) / n,
/// below 1 → 0, at least 0x7FFF → 0x7FFF. The path max distance
/// (n capped at 77) is not modelled.
fn flame_range(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    missile: UnitKey,
    m: i32,
    skill: u16,
    level: i32,
) -> Result<(), HandlerError> {
    let row = &inputs.tables.missiles[m as usize];
    let mut n = calc1(w, inputs, key, skill, level)?;
    if n <= 0 {
        n = row.param[1] + level - 1;
    }
    let n = n.clamp(1, 0xFF);
    let speed = ((i32::from(row.anim_len) << 8) / n).clamp(0, 0x7FFF);
    if let Some(mis) = w.objclient.missiles.get_mut(&missile) {
        mis.total = n;
        mis.current = n;
        mis.anim_speed = speed;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::client_do::client_do;
    use super::*;
    use crate::bridge::client_missiles::ClientMissileRow;
    use crate::bridge::critter_path::{self, Who};
    use crate::bridge::world::{ClientUnit, MonsterSequence, SkillRow, MISSILE, PLAYER};
    use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
    use std::sync::Arc;

    const SKILL: u16 = 5;
    const A: i16 = 2;
    const B: i16 = 3;

    fn inputs(cltdofunc: i16) -> ModelInputs {
        let mut i = ModelInputs::default();
        i.tables.skills = vec![SkillRow::default(); 8];
        i.tables.skills[usize::from(SKILL)] = SkillRow {
            ingame: true,
            cltdofunc,
            cltmissile: -1,
            cltmissile_abc: [A, B, -1],
            cltcalc1: u32::MAX,
            ..SkillRow::default()
        };
        i.tables.missiles = vec![ClientMissileRow::default(); 4];
        for m in [A, B] {
            i.tables.missiles[m as usize] = ClientMissileRow {
                vel: 6,
                range: 30,
                param: [0, 10],
                anim_len: 15,
                ..ClientMissileRow::default()
            };
        }
        i.skill_tables = Some(tables(0, u32::MAX, u32::MAX));
        i
    }

    /// Skill tables whose code is `07 n 00` (the constant n): `calc1`,
    /// `aurarangecalc` and `cltcalc1` at offsets `c1`, `ar`, `cc` (0 reads
    /// n, `u32::MAX` reads 0).
    fn tables_n(n: u8, c1: u32, ar: u32, cc: u32) -> Arc<d2_sim::skills::SkillTables> {
        fn blank<T: Record>() -> T {
            T::decode(&vec![0u8; T::SIZE])
        }
        let mut row: Skills = blank();
        (row.calc1, row.aurarangecalc, row.cltcalc1) = (c1, ar, cc);
        let mut skills = vec![blank::<Skills>(); 8];
        skills[usize::from(SKILL)] = row;
        Arc::new(d2_sim::skills::SkillTables {
            skills,
            skilldesc: vec![blank::<Skilldesc>()],
            missiles: vec![blank::<Missiles>()],
            skills_code: vec![0x07, n, 0x00],
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        })
    }

    fn tables(c1: u32, ar: u32, cc: u32) -> Arc<d2_sim::skills::SkillTables> {
        tables_n(0, c1, ar, cc)
    }

    /// A monster at (5000, 5000) using [`SKILL`], its client path aimed at
    /// `to`.
    fn monster(to: (u16, u16)) -> (ClientWorld, UnitKey) {
        let mut w = ClientWorld::default();
        let key = UnitKey::new(MONSTER, 2);
        let mut u = ClientUnit::new(key);
        u.position = Some((5000, 5000));
        u.used_skill = Some(i32::from(SKILL));
        u.used_skill_level = 2;
        w.units.insert(key, u);
        w.objclient.s_paths.insert(
            key,
            d2_sim::path::record::DynamicPath {
                target_x: to.0,
                target_y: to.1,
                direction: 16,
                precise_x: (5000 << 16) | 0x8000,
                precise_y: (5000 << 16) | 0x8000,
                ..Default::default()
            },
        );
        (w, key)
    }

    fn sequence(w: &mut ClientWorld, key: UnitKey, pos: i32) {
        w.units.get_mut(&key).unwrap().sequence = Some(MonsterSequence {
            frames: Vec::new(),
            length: 0x1000,
            pos,
            step: 0x100,
            drawn_mode: 14,
        });
    }

    fn step(w: &ClientWorld, key: UnitKey) -> i32 {
        w.units[&key].sequence.as_ref().unwrap().step
    }

    /// (start cell, target point) of every client missile, sorted.
    fn made(w: &ClientWorld) -> Vec<((u16, u16), (i32, i32))> {
        let mut v: Vec<_> = w
            .objclient
            .set_c
            .iter()
            .filter(|(k, _)| k.unit_type == MISSILE)
            .map(|(k, u)| (u.position.unwrap(), w.objclient.missiles[k].target_point))
            .collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn the_maggot_and_submerge_dos_stop_the_sequence() {
        let (mut w, key) = monster((5004, 5000));
        sequence(&mut w, key, 0x300);
        assert_eq!(client_do(&mut w, &inputs(46), key, SKILL, 2).unwrap(), 1);
        assert_eq!(step(&w, key), 0);
        assert!(!w.units[&key].flag_40);
        for f in [47, 52] {
            // Frame 1: the event byte cleared, the step kept.
            let (mut w, key) = monster((5004, 5000));
            sequence(&mut w, key, 0x100);
            let u = w.units.get_mut(&key).unwrap();
            (u.frame, u.action_frame) = (0x180, 4);
            assert_eq!(client_do(&mut w, &inputs(f), key, SKILL, 2).unwrap(), 1);
            assert_eq!((w.units[&key].action_frame, step(&w, key)), (0, 0x100));
            // Frame 0: the step := 0.
            w.units.get_mut(&key).unwrap().frame = 0xFF;
            client_do(&mut w, &inputs(f), key, SKILL, 2).unwrap();
            assert_eq!(step(&w, key), 0);
        }
    }

    #[test]
    fn the_spray_aims_at_the_tables_offsets_from_the_caster() {
        for (frame, second) in [(0x400, true), (0x800, false), (0x1000, true)] {
            let (mut w, key) = monster((5010, 5000));
            w.units.get_mut(&key).unwrap().frame = frame;
            assert_eq!(client_do(&mut w, &inputs(48), key, SKILL, 2).unwrap(), 1);
            let b = dir8(i32::from(dir64_to(&w, key, (5010, 5000)).unwrap())) as usize;
            let (mut x, mut y) = pair(usize::from(SPRAY_FIRST[b]));
            let i = ((frame >> 8) - 4).clamp(0, 8) as usize;
            assert_eq!(SPRAY_SECOND[b][i] != 99, second);
            if second {
                let (dx, dy) = pair(usize::from(SPRAY_SECOND[b][i]));
                (x, y) = (x + dx, y + dy);
            }
            assert_eq!(made(&w), vec![((5000, 5000), (5000 + x, 5000 + y))]);
        }
        // No used skill: nothing.
        let (mut w, key) = monster((5010, 5000));
        w.units.get_mut(&key).unwrap().used_skill = None;
        assert_eq!(client_do(&mut w, &inputs(48), key, SKILL, 2).unwrap(), 0);
        assert!(made(&w).is_empty());
    }

    #[test]
    fn the_swarm_move_sets_the_sequence_position_from_calc1() {
        let (mut w, key) = monster((5004, 5000));
        sequence(&mut w, key, 0x3A7);
        let mut i = inputs(49);
        i.skill_tables = Some(tables_n(11, 0, u32::MAX, u32::MAX));
        assert_eq!(client_do(&mut w, &i, key, SKILL, 2).unwrap(), 1);
        assert_eq!(w.units[&key].sequence.as_ref().unwrap().pos, 0xBA7);
    }

    #[test]
    fn the_nest_do_without_its_point_returns_0() {
        let (mut w, key) = monster((5004, 5000));
        assert_eq!(client_do(&mut w, &inputs(50), key, SKILL, 2).unwrap(), 0);
        assert!(!w.units[&key].flag_40 && made(&w).is_empty());
    }

    #[test]
    fn the_gargoyle_trap_shoots_along_the_longer_axis_and_turns() {
        let (mut w, key) = monster((0, 0));
        assert_eq!(client_do(&mut w, &inputs(51), key, SKILL, 2).unwrap(), 0);
        assert!(w.units[&key].flag_40);
        for (t, p, start) in [
            // |dx| ≥ |dy|: P = (T.x, U.y stepped to T.y).
            ((5010, 5002), (5010, 5002), (5000, 4999)),
            // |dx| < |dy|: P = (U.x stepped 4 toward T.x, T.y).
            ((4995, 5010), (4996, 5010), (4999, 5000)),
        ] {
            let (mut w, key) = monster((0, 0));
            let target = UnitKey::new(PLAYER, 1);
            let mut u = ClientUnit::new(target);
            u.position = Some(t);
            w.units.insert(target, u);
            critter_path::set_target_unit(&mut w, Who::server(key), Some(target));
            assert_eq!(client_do(&mut w, &inputs(51), key, SKILL, 2).unwrap(), 1);
            let (px, py) = p;
            let rel = (px - 5000, py - 5000);
            let s = (i32::from(start.0), i32::from(start.1));
            assert_eq!(made(&w), vec![(start, (s.0 + rel.0, s.1 + rel.1))]);
            let d = dir64_to(&w, key, (px, py)).unwrap();
            let want = TRAP_DIRECTION[usize::from(TRAP_QUADRANT[usize::from(d)] & 3)];
            assert_eq!(critter_path::direction(&w, Who::server(key)), Some(want));
        }
    }

    #[test]
    fn the_fetish_aura_makes_a_disc_of_aurarange_minus_6() {
        let (mut w, key) = monster((5004, 5000));
        let mut i = inputs(53);
        // aurarangecalc 8 → r 2: 13 points; no chance (cltcalc1 empty).
        i.skill_tables = Some(tables_n(8, u32::MAX, 0, u32::MAX));
        assert_eq!(client_do(&mut w, &i, key, SKILL, 2).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let got: Vec<_> = made(&w).into_iter().map(|(p, _)| p).collect();
        let mut want = Vec::new();
        for y in -2i32..=2 {
            for x in -2i32..=2 {
                if x * x + y * y <= 4 {
                    want.push(((5004 + x) as u16, (5000 + y) as u16));
                }
            }
        }
        want.sort_unstable();
        assert_eq!(got, want);
        assert_eq!(w.units[&key].seed, ClientUnit::new(key).seed);
    }

    #[test]
    fn the_fetish_inferno_makes_two_ranged_flames_on_the_casters_seed() {
        let (mut w, key) = monster((5004, 5000));
        let seed0 = w.units[&key].seed.unwrap();
        assert_eq!(client_do(&mut w, &inputs(54), key, SKILL, 2).unwrap(), 1);
        // Per flame: the class roll (b > 0), the light roll, the velocity
        // roll.
        let mut s = d2_sim::rng::Seed::new(seed0.0, seed0.1);
        let mut classes = Vec::new();
        for _ in 0..2 {
            classes.push(if s.roll(2) != 0 { B } else { A });
            s.roll(4);
            s.roll(5);
        }
        assert_eq!(w.units[&key].seed, Some((s.lo, s.hi)));
        let mut got: Vec<_> = w
            .objclient
            .set_c
            .iter()
            .filter(|(k, _)| k.unit_type == MISSILE)
            .map(|(_, u)| u.class as i16)
            .collect();
        got.sort_unstable();
        classes.sort_unstable();
        assert_eq!(got, classes);
        // Param2 10 + level 2 − 1 = 11 (not `Range` 30); speed 15 · 256 /
        // 11.
        let ms: Vec<_> = w.objclient.missiles.values().collect();
        assert!(ms
            .iter()
            .all(|m| (m.total, m.current, m.anim_speed) == (11, 11, 3840 / 11)));
        assert_eq!(made(&w).len(), 2);
        assert!(made(&w)
            .iter()
            .all(|&(p, t)| p == (5000, 5000) && t == (5004, 5000)));
        // No b: no class roll.
        let (mut w, key) = monster((5004, 5000));
        let mut i = inputs(54);
        i.tables.skills[usize::from(SKILL)].cltmissile_abc[1] = -1;
        client_do(&mut w, &i, key, SKILL, 2).unwrap();
        let mut s = d2_sim::rng::Seed::new(seed0.0, seed0.1);
        for _ in 0..4 {
            s.roll(4);
        }
        assert_eq!(w.units[&key].seed, Some((s.lo, s.hi)));
    }

    #[test]
    fn diablo_light_fires_one_flame_per_call_and_holds_the_frame() {
        use crate::bridge::skills::{SkillEntry, SkillList};
        use crate::bridge::world::MonsterClass;
        let with_entry = |w: &mut ClientWorld, key: UnitKey, param: i32| {
            w.units.get_mut(&key).unwrap().skills = Some(SkillList {
                entries: vec![SkillEntry {
                    skill: SKILL,
                    param,
                    ..SkillEntry::default()
                }],
                current: Some(0),
                ..SkillList::default()
            });
        };
        let mut i = inputs(56);
        i.tables.monsters = vec![Some(MonsterClass {
            inferno_anim: 9,
            ..MonsterClass::default()
        })];
        i.tables.skills[usize::from(SKILL)].monanim = 7;
        let (mut w, key) = monster((5004, 5000));
        with_entry(&mut w, key, 3);
        w.units.get_mut(&key).unwrap().frame = 12 << 8;
        let seed0 = w.units[&key].seed.unwrap();
        assert_eq!(client_do(&mut w, &i, key, SKILL, 2).unwrap(), 1);
        // One flame: the light roll, then the velocity roll (the DiabLight
        // table).
        let mut s = d2_sim::rng::Seed::new(seed0.0, seed0.1);
        s.roll(4);
        s.roll(5);
        assert_eq!(w.units[&key].seed, Some((s.lo, s.hi)));
        assert_eq!(made(&w), vec![((5000, 5000), (5004, 5000))]);
        // The param resets; the frame is held at InfernoAnim − 1.
        let u = &w.units[&key];
        assert_eq!(u.skills.as_ref().unwrap().entries[0].param, 0);
        assert_eq!(u.frame, 8 << 8);
        // A sequence anim (monanim 14): position 8, count 10 frames.
        i.tables.skills[usize::from(SKILL)].monanim = 14;
        let (mut w, key) = monster((5004, 5000));
        with_entry(&mut w, key, 0);
        sequence(&mut w, key, 0x20);
        client_do(&mut w, &i, key, SKILL, 2).unwrap();
        let u = &w.units[&key];
        assert_eq!(
            (u.sequence.as_ref().unwrap().pos, u.frame_count),
            (0x820, 10 << 8)
        );
        // An interval (`cltcalc1` 3): calls count up and make nothing
        // until the param reaches it.
        i.skill_tables = Some(tables_n(3, u32::MAX, u32::MAX, 0));
        let (mut w, key) = monster((5004, 5000));
        with_entry(&mut w, key, 0);
        assert_eq!(client_do(&mut w, &i, key, SKILL, 2).unwrap(), 0);
        assert_eq!(w.units[&key].skills.as_ref().unwrap().entries[0].param, 1);
        assert!(made(&w).is_empty());
        // No used entry: nothing.
        let (mut w, key) = monster((5004, 5000));
        assert_eq!(client_do(&mut w, &i, key, SKILL, 2).unwrap(), 0);
    }

    #[test]
    fn the_inferno_skill_step_repeats_the_do_and_rolls_the_frame_back() {
        use crate::bridge::world::MonsterClass;
        let (mut w, key) = monster((5004, 5000));
        let mut i = inputs(54);
        i.tables.skills[usize::from(SKILL)].srvdofunc = 95;
        i.tables.monsters = vec![Some(MonsterClass {
            inferno_anim: 12,
            inferno_rollback: 7,
            ..MonsterClass::default()
        })];
        // No event byte: the do still runs; frame 12 is kept.
        w.units.get_mut(&key).unwrap().frame = 12 << 8;
        crate::bridge::monster_anim::skill_step(&mut w, &i, key).unwrap();
        assert_eq!((made(&w).len(), w.units[&key].frame), (2, 12 << 8));
        // Frame 13: the do, then back to frame 7.
        w.units.get_mut(&key).unwrap().frame = 13 << 8;
        crate::bridge::monster_anim::skill_step(&mut w, &i, key).unwrap();
        assert_eq!((made(&w).len(), w.units[&key].frame), (4, 7 << 8));
        // No monstats2 row: nothing.
        i.tables.monsters = vec![None];
        crate::bridge::monster_anim::skill_step(&mut w, &i, key).unwrap();
        assert_eq!(made(&w).len(), 4);
    }
}
