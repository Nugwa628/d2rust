//! The client skill do: the player update runs it on an action frame
//! of a skill mode, the monster update's
//! skill step (`monster_anim::skill_step`) likewise, and its `cltdofunc`
//! (table, 130 entries) makes the client's own effects of
//! the skill: for the novas (function 25) the 64 client
//! missiles of the ring, whose `TravelSound` the client
//! missile create requests; for Teeth (function 17) the
//! fan of `calc1` missiles across the cast target; Fire Wall (26), Chain
//! Lightning (27), Meteor / Blizzard (28) and Frozen Orb (29) their
//! missiles at or toward the cast target; Psychic Hammer (3), Dragon
//! Tail (7), Guided Arrow / Bone Spirit (18) and Charged Strike (19),
//! Jab (16, sound only), Find Potion / Find Item (38 / 40, the blood
//! spray on the target corpse), Grim Ward (41, its ward and the corpse
//! explosion at the target corpse); the monster towers' DesertTurret
//! (60) fan and ArcaneTower (61) ring; the monster functions 55 (Prime
//! Poison Nova), 57 (Finger Mage Spider), 58 (DiabWall) and 63
//! (Regurgitator Eat) theirs; Throw (function 2) the
//! worn weapon's
//! `missiletype` toward the cast target;
//! Firestorm, Twister and Tornado (67–69) `calc1` bolt-pathed missiles
//! toward it; the monster functions 46–54 ([`super::client_do_mon`]);
//! the Doom Knight and Necromage missiles (94, 95) one missile chosen
//! by a monster component toward the target; the skill's
//! `cltmissile` (fire bolt)
//! is one client missile toward the cast target. The view draws both
//! (`world_view::missiles`). Each function has its own module.
//!
//! Function 5 (Shock Field) scatters lobbed client
//! missiles around the cast target; the client start function 19
//! (Teeth) makes the `cltmissilec` cast missile
//! ([`bone_cast`]).
//!
//! Lightning Strike (22) and Fist of the Heavens (36) aim at the path
//! target unit; the curses (30) make a `cursecenter` with a red light
//! at the cast target; Blessed Hammer (35) lays its spiral point path.
//! Raise (31) creates its missile at the corpse the start wrote into the
//! used entry; Corpse Explosion (32) sprays by the corpse's size, then b
//! and c; Poison Explosion (33) makes a, c and the ring of b; Sacrifice
//! (34) makes its missile behind U.
//!
//! Not modelled (each named where it would run): the progressive lob of
//! and of functions 2, 94 and 95, the functions other than 2–8, 16–36, 38–41, 46–55, 57, 58, 60, 61, 62, 63, 67–70, 73–79, 81, 94 and 95 (left as they were: no effect, flag 0x40
//! untouched; PROVISIONAL).

mod f001_attack;
mod f002_throw;
mod f003_psychic_hammer;
mod f004_dragon_talon;
mod f005_shock_field;
mod f006_blade_fury;
mod f007_dragon_tail;
mod f008_mind_blast;
mod f016_jab;
mod f017_teeth;
mod f018_guided;
mod f019_charged_strike;
mod f020_strafe;
mod f021_melee_repeat;
mod f022_lightning_strike;
mod f023_charged_bolt;
mod f024_inferno;
mod f025_nova;
mod f026_fire_wall;
mod f027_chain_lightning;
mod f028_meteor;
mod f029_frozen_orb;
mod f030_curse;
mod f031_raise;
mod f032_f033_corpse_explosion;
mod f034_sacrifice;
mod f035_blessed_hammer;
mod f036_fist_of_heavens;
mod f037_charge;
mod f038_f040_find;
mod f039_swing;
mod f041_grim_ward;
mod f042_double_throw;
mod f044_leap_attack;
mod f045_whirlwind;
mod f055_prime_poison_nova;
mod f057_finger_mage_spider;
mod f058_diablo_wall;
mod f059_diablo_run;
mod f060_desert_turret;
mod f061_arcane_tower;
mod f062_mosquito;
mod f063_regurgitator_eat;
mod f064_queen_death;
mod f066_missile_skill;
mod f067_f069_calc1_spread;
mod f070_wake_maker;
mod f072_imp_fireball;
mod f073_teleport_end;
mod f074_imp_teleport;
mod f075_siege_beast_stomp;
mod f076_death_maul;
mod f077_inferno_sentry;
mod f078_f079_corpse_sentries;
mod f081_baal_cold_maker;
mod f094_f095_component;
#[cfg(test)]
mod test_util;

use super::client_missiles::{create, flag, CreateRecord, MissileSound};
use super::dispatch::HandlerError;
use super::msg::skills::level_with_bonuses;
use super::world::{
    CastTarget, ClientWorld, KindData, ModelInputs, SkillRow, UnitKey, MONSTER, PLAYER,
};
use crate::rules::lighting::records::LightKind;

/// Entries of the do table.
pub const TABLE_LEN: i16 = 130;

/// The null entries of the do table: 0 and 97…129.
fn null_entry(f: i16) -> bool {
    f == 0 || (97..TABLE_LEN).contains(&f)
}

/// The class whose ring toggles record flag 0x4000 per missile.
const RING_TOGGLE_CLASS: u32 = 176;

/// With (U): the used skill of U with its level with bonuses
/// → [`client_do`] with w = 0. No used skill:
/// nothing.
pub fn generic_do(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<(), HandlerError> {
    let Some(e) = w
        .units
        .get(&key)
        .and_then(|u| u.skills.as_ref())
        .and_then(|l| l.current.and_then(|i| l.entries.get(i)))
        .copied()
    else {
        return Ok(());
    };
    let t = &inputs.tables;
    let level = level_with_bonuses(w, key, &t.skills, &t.skilldesc, &e);
    client_do(w, inputs, key, e.skill, level).map(|_| ())
}

/// The client do with (U, skill, level, 0). Returns
/// its result (1 = the do ran).
pub fn client_do(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let f = row.cltdofunc;
    if !(0..TABLE_LEN).contains(&f) {
        return Ok(0);
    }
    // The `cltmissile` create (w = 0: no offsets, the
    // target); flag 0x40 on U either way.
    let mut made = false;
    if row.cltmissile > 0 {
        made = if row.lob {
            // The `lob` row (byte +4 bit 0x02): with no
            // offsets and the target, arc 0 (→ 30).
            lob(w, inputs, key, row.cltmissile as u32, (0, 0), skill, level)?
        } else {
            cast_missile(w, inputs, key, skill, level)?
        };
        set_flag_40(w, key);
    }
    // The function (w = 0: no `ItemTgtDo` replacement).
    let mut ran = if null_entry(f) {
        set_flag_40(w, key);
        1
    } else {
        match f {
            1 => f001_attack::attack(w, inputs, key, skill, level)?,
            2 => f002_throw::throw(w, inputs, key, skill)?,
            3 => f003_psychic_hammer::psychic_hammer(w, inputs, key)?,
            5 => f005_shock_field::shock_field(w, inputs, key, skill, level)?,
            4 => f004_dragon_talon::dragon_talon(w, key),
            7 => f007_dragon_tail::dragon_tail(w, inputs, key, skill, level)?,
            8 => f008_mind_blast::mind_blast(w, inputs, key, skill, level)?,
            16 => f016_jab::jab(w, inputs, key, skill),
            17 => f017_teeth::spread(w, inputs, key, skill, level)?,
            18 => f018_guided::guided(w, inputs, key, skill, level)?,
            19 => f019_charged_strike::charged_strike(w, inputs, key, skill, level)?,
            20 => f020_strafe::strafe(w, inputs, key, skill, level)?,
            21 => f021_melee_repeat::melee_repeat(w, inputs, key, skill)?,
            22 => f022_lightning_strike::lightning_strike(w, inputs, key, skill, level)?,
            23 => f023_charged_bolt::charged_bolt(w, inputs, key, skill, level)?,
            6 => f006_blade_fury::blade_fury(w, inputs, key, skill, level)?,
            24 => f024_inferno::inferno(w, inputs, key, skill, level)?,
            25 => f025_nova::nova(w, inputs, key, skill, level)?,
            26 => f026_fire_wall::fire_wall(w, inputs, key, skill, level)?,
            27 => f027_chain_lightning::chain_lightning(w, inputs, key, skill, level)?,
            28 => f028_meteor::meteor(w, inputs, key, skill, level)?,
            29 => f029_frozen_orb::frozen_orb(w, inputs, key, skill, level)?,
            30 => f030_curse::curse(w, inputs, key, skill, level)?,
            31 => f031_raise::raise(w, inputs, key, skill, level)?,
            32 => f032_f033_corpse_explosion::corpse_explosion(w, inputs, key, skill, level)?,
            33 => f032_f033_corpse_explosion::poison_explosion(w, inputs, key, skill, level)?,
            34 => f034_sacrifice::sacrifice(w, inputs, key, skill, level)?,
            35 => f035_blessed_hammer::blessed_hammer(w, inputs, key, skill, level)?,
            36 => f036_fist_of_heavens::fist_of_heavens(w, inputs, key, skill, level)?,
            37 => f037_charge::charge(w, key)?,
            44 => f044_leap_attack::leap_attack(w, inputs, key)?,
            45 => f045_whirlwind::whirlwind(w, key),
            38 => f038_f040_find::find_potion(w, inputs, key)?,
            39 => f039_swing::swing(w, inputs, key)?,
            40 => f038_f040_find::find_item(w, inputs, key)?,
            41 => f041_grim_ward::grim_ward(w, inputs, key, skill, level)?,
            42 => f042_double_throw::double_throw(w, inputs, key, skill, level)?,
            46 => super::client_do_mon::maggot_egg(w, key),
            47 | 52 => super::client_do_mon::submerge_frame(w, key),
            48 => super::client_do_mon::andariel_spray(w, inputs, key, skill, level)?,
            49 => super::client_do_mon::swarm_move(w, inputs, key, skill, level)?,
            // Nest: its point is the used skill
            // entry's params 2 and 3, which read 0 here: 0.
            50 => 0,
            51 => super::client_do_mon::gargoyle_trap(w, inputs, key, skill, level)?,
            53 => super::client_do_mon::fetish_aura(w, inputs, key, skill, level)?,
            54 => super::client_do_mon::fetish_inferno(w, inputs, key, skill, level)?,
            55 => f055_prime_poison_nova::prime_poison_nova(w, inputs, key, skill, level)?,
            56 => super::client_do_mon::diablo_light(w, inputs, key, skill, level)?,
            57 => f057_finger_mage_spider::finger_mage_spider(w, inputs, key, skill, level)?,
            58 => f058_diablo_wall::diablo_wall(w, inputs, key, skill, level)?,
            59 => f059_diablo_run::diablo_run(w, inputs, key, skill)?,
            60 => f060_desert_turret::desert_turret(w, inputs, key, skill, level)?,
            61 => f061_arcane_tower::arcane_tower(w, inputs, key, skill, level)?,
            62 => f062_mosquito::mosquito(w, inputs, key, skill)?,
            63 => f063_regurgitator_eat::regurgitator_eat(w, inputs, key, skill, level)?,
            64 => f064_queen_death::queen_death(w, inputs, key)?,
            66 => f066_missile_skill::missile_skill(w, inputs, key, skill, level)?,
            67 => f067_f069_calc1_spread::calc1_spread(
                w,
                inputs,
                key,
                skill,
                level,
                f067_f069_calc1_spread::FIRESTORM,
            )?,
            68 => f067_f069_calc1_spread::calc1_spread(
                w,
                inputs,
                key,
                skill,
                level,
                f067_f069_calc1_spread::TWISTER,
            )?,
            69 => f067_f069_calc1_spread::calc1_spread(
                w,
                inputs,
                key,
                skill,
                level,
                f067_f069_calc1_spread::TORNADO,
            )?,
            70 => f070_wake_maker::wake_maker(w, inputs, key, skill, level)?,
            73 => f073_teleport_end::teleport_end(w, inputs, key, skill)?,
            72 => f072_imp_fireball::imp_fireball(w, inputs, key, skill, level)?,
            74 => f074_imp_teleport::imp_teleport(w, inputs, key, skill)?,
            75 => f075_siege_beast_stomp::siege_beast_stomp(w, inputs, skill),
            76 => f076_death_maul::death_maul(w, inputs, key, skill, level)?,
            77 => f077_inferno_sentry::inferno_sentry(w, inputs, key, skill, level)?,
            78 => f078_f079_corpse_sentries::death_sentry(w, inputs, key, skill, level)?,
            79 => f078_f079_corpse_sentries::fenris_rage(w, inputs, key, skill, level)?,
            81 => f081_baal_cold_maker::baal_cold_maker(w, inputs, key, skill, level)?,
            94 => f094_f095_component::component_missile(w, inputs, key, skill, level, 10)?,
            95 => f094_f095_component::component_missile(w, inputs, key, skill, level, 11)?,
            // PROVISIONAL: the other functions are not modelled.
            _ => return Ok(0),
        }
    };
    ran |= i32::from(made);
    // The unit's cast light is detached and removed either way.
    super::modes::remove_unit_light(w, key);
    if ran == 0 {
        return Ok(0);
    }
    // The sound tail.
    sound_tail(w, inputs, key, &row);
    skill_lock(w, inputs, key, skill, level, &row)?;
    Ok(1)
}

/// The do tail's skill-use lock: d := the skill's
/// `delay` calc (row +0x190); d > 0, U the local player
/// and (the row's `anim` (+0x10) is not 0x12, or U's event
/// index (+0x38 >> 8) is 0) → the lock := C + min(d, 12)
/// (C =).
fn skill_lock(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
    row: &SkillRow,
) -> Result<(), HandlerError> {
    // The calc runs first in the base game; it has no model effect, so the
    // local-player test may go first.
    if w.local_player != Some(key) {
        return Ok(());
    }
    let d = skill_calc(w, inputs, key, skill, level, |r| r.delay)?;
    if d <= 0 {
        return Ok(());
    }
    let event = w.units.get(&key).map_or(0, |u| u.event_index);
    if row.anim != 0x12 || event == 0 {
        w.skill_lock = w.drlg_updates.wrapping_add(d.min(12) as u32);
    }
    Ok(())
}

/// The do's tail when it ran: `dosound` > 0 → a request of it
/// on U; with T := U's path target unit,
/// `tgtsound` > 0 → a request of it on T, and
/// `tgtoverlay` > 0 → the overlay with (T, o, 2, 0…). The requests go to the
/// audio layer in the client missiles' sound list, after the sounds of
/// any missile the do made. The local player's skill-use lock follows
/// ([`skill_lock`]).
fn sound_tail(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, row: &SkillRow) {
    if row.dosound > 0 {
        w.objclient.missile_sounds.push(MissileSound::Request {
            id: i32::from(row.dosound),
            missile: key,
        });
    }
    if row.tgtsound > 0 {
        if let Some(t) = path_target_unit(w, key) {
            w.objclient.missile_sounds.push(MissileSound::Request {
                id: i32::from(row.tgtsound),
                missile: t,
            });
        }
    }
    if row.tgtoverlay > 0 {
        if let Some(t) = path_target_unit(w, key) {
            super::overlay_lights::create(w, inputs, t, row.tgtoverlay as u16, 2);
        }
    }
}

/// With (class = cltmissile, U, skill, level, 0, 0, 0, 0): a
/// record with flags 0x21 (start position + absolute target), owner U,
/// start = U's position, target from [`cast_target_point`]; none made
/// when a target coordinate stays 0. The result counts as ran.
fn cast_missile(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<bool, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)) else {
        return Ok(false);
    };
    let class = row.cltmissile;
    let Some(u) = w.units.get(&key) else {
        return Ok(false);
    };
    let (x, y) = u.cell();
    let (tx, ty) = cast_target_point(w, key);
    if tx == 0 || ty == 0 {
        return Ok(false);
    }
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE,
        owner: Some(key),
        class: class as u32,
        x: i32::from(x),
        y: i32::from(y),
        tx,
        ty,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    Ok(create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?.is_some())
}

/// With (T): T a monster whose `monstats2` has `soft`
/// (monstats2 flag 0xC); its class row.
fn soft_monster(
    w: &ClientWorld,
    inputs: &ModelInputs,
    t: UnitKey,
) -> Option<super::world::MonsterClass> {
    if t.unit_type != MONSTER {
        return None;
    }
    let u = w.units.get(&t)?;
    inputs
        .tables
        .monsters
        .get(u.class as usize)
        .copied()
        .flatten()
        .filter(|c| c.soft)
}

/// U's path direction (0…63) for the aim nudge: a monster's client path
/// (`critter_path`), a player's `path_dir`.
pub(super) fn owner_dir64(w: &ClientWorld, key: UnitKey) -> Option<u8> {
    if key.unit_type == MONSTER {
        super::critter_path::direction(w, super::critter_path::Who::server(key))
    } else {
        w.units.get(&key).and_then(|u| u.path_dir)
    }
}

/// With (U): the target unit's position, else the path's target
/// point; a zero coordinate takes
/// the aim nudge (U's position + the direction step).
pub(super) fn cast_target_point(w: &ClientWorld, key: UnitKey) -> (i32, i32) {
    const DX: [i32; 8] = [0, -1, -2, -1, 0, 1, 2, 1];
    const DY: [i32; 8] = [2, 1, 0, -1, -2, -1, 0, 1];
    let Some(u) = w.units.get(&key) else {
        return (0, 0);
    };
    // A monster's path is its client path (`critter_path`): its target
    // unit, else its target point; the facing is the path's direction.
    if key.unit_type == MONSTER {
        let who = super::critter_path::Who::server(key);
        let (x, y) = match super::critter_path::target_unit(w, who) {
            Some(t) => w.units.get(&t).and_then(|t| t.position),
            None => super::critter_path::target(w, who),
        }
        .map_or((0, 0), |(x, y)| (i32::from(x), i32::from(y)));
        if x != 0 && y != 0 {
            return (x, y);
        }
        let d = usize::from(super::critter_path::direction(w, who).unwrap_or(0) >> 3);
        let (cx, cy) = u.cell();
        return (i32::from(cx) + DX[d], i32::from(cy) + DY[d]);
    }
    let (x, y) = match u.cast_target {
        Some(CastTarget::Unit(t)) => w
            .units
            .get(&t)
            .and_then(|t| t.position)
            .map_or((0, 0), |(x, y)| (i32::from(x), i32::from(y))),
        Some(CastTarget::Point(x, y)) => (i32::from(x), i32::from(y)),
        None => (0, 0),
    };
    if x != 0 && y != 0 {
        return (x, y);
    }
    let d = usize::from(u.path_dir.unwrap_or(0) >> 3);
    let (cx, cy) = u.cell();
    (i32::from(cx) + DX[d], i32::from(cy) + DY[d])
}

pub(super) fn set_flag_40(w: &mut ClientWorld, key: UnitKey) {
    if let Some(u) = w.units.get_mut(&key) {
        u.flag_40 = true;
    }
}

/// The hand class with (U): the type class of U's
/// weapon with (U, 1), 0 without one. The model reads it from the
/// player's COF weapon class (`player_anim::type_class`: `bow` 1 …
/// `xbw` 7, `ht1` 12); other units carry no weapon item, 0.
fn hand_class(w: &ClientWorld, inputs: &ModelInputs, key: UnitKey) -> i32 {
    if key.unit_type != PLAYER {
        return 0;
    }
    inputs
        .player_anims
        .as_ref()
        .and_then(|a| a.weapon(w, key))
        .map_or(0, |c| super::player_anim::type_class(c) as i32)
}

/// With (U, class, tx, ty, skill, level):
/// record flags 0x20 (absolute target), owner U (a player or
/// monster here), origin U.
pub(super) fn create_toward(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    class: i32,
    (tx, ty): (i32, i32),
    skill: u16,
    level: i32,
) -> Result<Option<UnitKey>, HandlerError> {
    let rec = CreateRecord {
        flags: flag::TARGET_ABSOLUTE,
        owner: Some(key),
        origin: Some(key),
        class: class as u32,
        tx,
        ty,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)
}

/// With (U, class, x, y, skill, level):
/// record flags 1 (start position (x, y)), owner U; no target.
fn create_at(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    class: i32,
    (x, y): (i32, i32),
    skill: u16,
    level: i32,
) -> Result<Option<UnitKey>, HandlerError> {
    let rec = CreateRecord {
        flags: flag::POSITION,
        owner: Some(key),
        class: class as u32,
        x,
        y,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)
}

/// With (U, class, skill, level): the target T :=
/// [`cast_target_point`]; both coordinates nonzero → [`create_toward`] T.
pub(super) fn cast_toward(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    class: i32,
    skill: u16,
    level: i32,
) -> Result<Option<UnitKey>, HandlerError> {
    let t = cast_target_point(w, key);
    if t.0 == 0 || t.1 == 0 {
        return Ok(None);
    }
    create_toward(w, inputs, key, class, t, skill, level)
}

/// With (U, class, skill, level): the target T :=
/// [`cast_target_point`]; both coordinates nonzero and |T − U|²
/// ≤ 10000 → [`create_at`] T.
fn cast_at(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    class: i32,
    skill: u16,
    level: i32,
) -> Result<Option<UnitKey>, HandlerError> {
    let (tx, ty) = cast_target_point(w, key);
    if tx == 0 || ty == 0 {
        return Ok(None);
    }
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(None);
    };
    let (dx, dy) = (tx - i32::from(ux), ty - i32::from(uy));
    if dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) > 10_000 {
        return Ok(None);
    }
    create_at(w, inputs, key, class, (tx, ty), skill, level)
}

/// With (U, class): U in a room, the target T :=
/// [`cast_target_point`] with both coordinates nonzero, and the
/// collision query with (U's room, T, Size, Size, 5) free (0).
fn target_free(w: &ClientWorld, inputs: &ModelInputs, key: UnitKey, class: i32) -> bool {
    let Some(row) = usize::try_from(class)
        .ok()
        .and_then(|i| inputs.tables.missiles.get(i))
    else {
        return false;
    };
    let (tx, ty) = cast_target_point(w, key);
    if tx == 0 || ty == 0 {
        return false;
    }
    super::client_missiles::unit_room_query(w, key, tx, ty, u32::from(row.size), 5) == Some(0)
}

pub(super) fn valid_missile(inputs: &ModelInputs, m: i32) -> bool {
    usize::try_from(m).is_ok_and(|i| i < inputs.tables.missiles.len())
}

/// The skill's `calc1` through the client calc evaluator
/// (skill row +0x138).
pub(super) fn calc1(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    skill_calc(w, inputs, key, skill, level, |r| r.calc1)
}

/// The skill's `cltcalc1` (row +0x114), as [`calc1`].
pub(super) fn cltcalc1(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    skill_calc(w, inputs, key, skill, level, |r| r.cltcalc1)
}

/// The skill's `calc2` (row +0x13C), as [`calc1`].
pub(super) fn calc2(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    skill_calc(w, inputs, key, skill, level, |r| r.calc2)
}

/// A calc field of the skill's row (`pick`) through the client calc
/// evaluator with (U, field, skill, level).
pub(super) fn skill_calc(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
    pick: fn(&d2_data::tables::Skills) -> u32,
) -> Result<i32, HandlerError> {
    let t = inputs.skill_tables.as_deref().ok_or(HandlerError::Invalid(
        "client/msg-skills.md §11: a skill calc needs the skills tables",
    ))?;
    let Some(srow) = t.skills.get(usize::from(skill)) else {
        return Ok(0);
    };
    let mut units = super::passive::ClientSkills::new(w, key);
    Ok(d2_sim::skills::levels::eval_skill(
        &mut units,
        t,
        Some(key),
        pick(srow),
        i32::from(skill),
        level,
    ))
}

/// The search with (U, 3, 3, …) of the Charge do:
/// the units of U's room and its neighbours, U excluded,
/// whose cell lies within distance² 9 of U's and that pass the filter
/// with mask 0xA283: a player not in mode 0 or 0x11, a
/// monster not in mode 0 or 0xC; no other unit type. The pick among
/// several (callback) is not in the exports: this returns
/// the first candidate only to tell "none" from "some".
fn charge_search(w: &ClientWorld, key: UnitKey) -> Option<UnitKey> {
    let (x, y) = w.units.get(&key)?.cell();
    let (x, y) = (i32::from(x), i32::from(y));
    w.units.values().find_map(|v| {
        let alive = match v.key.unit_type {
            PLAYER => !matches!(v.mode, 0 | 0x11),
            MONSTER => !matches!(v.mode, 0 | 0xC),
            _ => false,
        };
        let (vx, vy) = v.position?;
        let (dx, dy) = (i32::from(vx) - x, i32::from(vy) - y);
        (v.key != key && alive && dx * dx + dy * dy <= 9).then_some(v.key)
    })
}

/// With (U): the used skill entry of U.
fn current_entry(w: &ClientWorld, key: UnitKey) -> Option<&super::skills::SkillEntry> {
    let l = w.units.get(&key)?.skills.as_ref()?;
    l.entries.get(l.current?)
}

fn current_entry_mut(w: &mut ClientWorld, key: UnitKey) -> Option<&mut super::skills::SkillEntry> {
    let l = w.units.get_mut(&key)?.skills.as_mut()?;
    let i = l.current?;
    l.entries.get_mut(i)
}

/// With (U, skill, level): the skill's progressive count. A
/// `progressive` row with a valid `aurastate` and `aurastat1` and that
/// state's list on U evaluates `prgcalc` n (the list's stat clamped to
/// 1…3); every other case evaluates `prgcalc1`. Through the client calc
/// evaluator.
fn prog_calc(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let t = inputs.skill_tables.as_deref().ok_or(HandlerError::Invalid(
        "client/msg-skills.md §11: prgcalc needs the skills tables",
    ))?;
    let (Some(srow), Some(row)) = (
        t.skills.get(usize::from(skill)),
        inputs.tables.skills.get(usize::from(skill)),
    ) else {
        return Ok(0);
    };
    let mut c = srow.prgcalc1;
    if row.progressive && row.aurastate >= 0 && row.aurastat1 >= 0 {
        let n = u8::try_from(row.aurastate)
            .ok()
            .and_then(|s| w.units.get(&key)?.state_lists.get(&s))
            .map(|l| l.get(&(row.aurastat1 as u16, 0)).copied().unwrap_or(0));
        if let Some(n) = n {
            c = [srow.prgcalc1, srow.prgcalc2, srow.prgcalc3][(n.clamp(1, 3) - 1) as usize];
        }
    }
    let mut units = super::passive::ClientSkills::new(w, key);
    Ok(d2_sim::skills::levels::eval_skill(
        &mut units,
        t,
        Some(key),
        c,
        i32::from(skill),
        level,
    ))
}

/// With (&U.seed, n): one step of U's seed (+0x20), the roll of
/// n; a unit without a seed reads 0.
pub(crate) fn unit_roll(w: &mut ClientWorld, key: UnitKey, n: i32) -> u32 {
    let Some(u) = w.units.get_mut(&key) else {
        return 0;
    };
    let Some((lo, hi)) = u.seed else {
        return 0;
    };
    let mut seed = d2_sim::rng::Seed::new(lo, hi);
    let r = seed.roll(n);
    u.seed = Some((seed.lo, seed.hi));
    r
}

/// One raw step of U's seed (+0x20), its new low word; a unit without a
/// seed reads 0.
fn unit_step(w: &mut ClientWorld, key: UnitKey) -> u32 {
    let Some(u) = w.units.get_mut(&key) else {
        return 0;
    };
    let Some((lo, hi)) = u.seed else {
        return 0;
    };
    let mut seed = d2_sim::rng::Seed::new(lo, hi);
    let r = seed.step();
    u.seed = Some((seed.lo, seed.hi));
    r
}

/// With (U): the target unit of U's path: a monster's client
/// path (`critter_path`), a player's unit cast target.
pub(super) fn path_target_unit(w: &ClientWorld, key: UnitKey) -> Option<UnitKey> {
    if key.unit_type == MONSTER {
        return super::critter_path::target_unit(w, super::critter_path::Who::server(key));
    }
    match w.units.get(&key)?.cast_target {
        Some(CastTarget::Unit(t)) => Some(t),
        _ => None,
    }
}

/// The frame rewind with (U, p): U's frame (+0x44, 8.8) := ((100 − p) · (frame >>
/// 8) / 100) · 0x100, the animation rewound to (100 − p) % of its whole
/// frames (p = 100: the start).
fn rewind(frame: &mut i32, p: i32) {
    *frame = ((100 - p).wrapping_mul(*frame >> 8) / 100).wrapping_mul(0x100);
}

/// The [`super::client_missiles::Env`] of the client unit search
/// from the model inputs.
pub(super) fn search_env(inputs: &ModelInputs) -> super::client_missiles::Env<'_> {
    super::client_missiles::Env {
        rows: &inputs.tables.missiles,
        lights: inputs.high_light_quality,
        skills: inputs.skill_tables.as_deref(),
        monsters: &inputs.tables.monsters,
        model: Some(inputs),
    }
}

/// The path facing with T's position: U faces T's sub-tile, turning
/// by steps.
pub(super) fn face_unit(w: &mut ClientWorld, key: UnitKey, t: UnitKey) {
    if let Some((x, y)) = w.units.get(&t).map(|u| u.cell()) {
        super::critter_path::face_point(w, key, x, y);
    }
}

/// With (U) − 4: U's melee range (the client
/// search's reach, [`super::client_missiles::melee_reach`]).
pub(super) fn reach(
    w: &ClientWorld,
    env: &super::client_missiles::Env,
    key: UnitKey,
) -> Result<i32, HandlerError> {
    super::client_missiles::melee_reach(w, env, key)
}

/// Writes U's used skill entry through `f`.
fn set_entry(w: &mut ClientWorld, key: UnitKey, f: impl FnOnce(&mut super::skills::SkillEntry)) {
    if let Some(e) = w
        .units
        .get_mut(&key)
        .and_then(|u| u.skills.as_mut())
        .and_then(|l| l.current.and_then(|i| l.entries.get_mut(i)))
    {
        f(e);
    }
}

/// The monster side of the client skill start's `cltstfunc` step
/// (step 5, with U, skill and level). Function 48 (Teleport 2, Baal
/// Teleport, Imp Teleport, Baal Clone Teleport): the skill row not valid →
/// 0; m := `cltmissilea` (+0xEA, signed) in the missile table →
/// a create of m at U's position with (skill, level); 1. Functions 50
/// (mon death sentry) and 51 (fenris rage):
/// [`f078_f079_corpse_sentries`]; 52 (mon inferno sentry):
/// [`f077_inferno_sentry::inferno_sentry_start`]; 42
/// (Mosquito: the loop count and the target)
/// through the start table ([`super::use_state::client_start_apply`]).
/// Every other function: 1 (PROVISIONAL: the monster start runs
/// no other body).
pub fn monster_start(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    match row.cltstfunc {
        50 => return Ok(f078_f079_corpse_sentries::death_sentry_start(w, key)),
        51 => return Ok(f078_f079_corpse_sentries::fenris_rage_start(w, key)),
        52 => return f077_inferno_sentry::inferno_sentry_start(w, inputs, key, skill, level),
        42 => {
            super::use_state::client_start_apply(w, inputs, key, skill, level)?;
            return Ok(1);
        }
        32 => return magott_up_start(w, inputs, key, skill, level),
        _ => {}
    }
    if row.cltstfunc != 48 {
        return Ok(1);
    }
    let m = i32::from(row.cltmissile_abc[0]);
    if valid_missile(inputs, m) {
        if let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) {
            create_at(
                w,
                inputs,
                key,
                m,
                (i32::from(x), i32::from(y)),
                skill,
                level,
            )?;
        }
    }
    Ok(1)
}

/// `cltstfunc` 32 (MagottUp, skill 161):
/// the sequence is reloaded (the mode's animation from frame 0), unit
/// flags +0xC4 |= 0xE (bits 0x2 selectable and 0x4 attackable kept as `flag_2` / `flag_4`;
/// 0x8 is not modelled), and the row's `cltmissilea` (+0xEA) is created at the unit's cell
/// (missile 322 at the maggot). Then the
/// unit is placed again on the free point at its position (size, mask 0x1C09,
/// fallback 1) and the room's cell is marked 0x1000.
/// PROVISIONAL: the placement and the mark are not run (the
/// maggot stands on a free cell, so the point is its own).
fn magott_up_start(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).copied() else {
        return Ok(0);
    };
    let Some(mode) = w.units.get(&key).map(|u| u.mode) else {
        return Ok(0);
    };
    super::monster_anim::mode_init(w, inputs, key, mode);
    if let Some(u) = w.units.get_mut(&key) {
        u.flag_2 = Some(true);
        u.flag_4 = true;
    }
    let m = i32::from(row.cltmissile_abc[0]);
    if valid_missile(inputs, m) {
        if let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) {
            create_at(
                w,
                inputs,
                key,
                m,
                (i32::from(x), i32::from(y)),
                skill,
                level,
            )?;
        }
    }
    Ok(1)
}

/// With (U, skill): a `progressive` row with a valid
/// `aurastate` and `aurastat1` whose state list on U holds the stat at
/// n > 1 → missile b (n = 2) or c (n ≥ 3); else `cltmissilea`.
fn missile_class(w: &ClientWorld, key: UnitKey, row: &super::world::SkillRow) -> i32 {
    let [a, b, c] = row.cltmissile_abc.map(i32::from);
    if !row.progressive || row.aurastate < 0 || row.aurastat1 < 0 {
        return a;
    }
    let n = u8::try_from(row.aurastate)
        .ok()
        .and_then(|s| w.units.get(&key)?.state_lists.get(&s))
        .and_then(|l| l.get(&(row.aurastat1 as u16, 0)).copied())
        .unwrap_or(0);
    match n {
        2 => b,
        3.. => c,
        _ => a,
    }
}

/// The ring with (U, U, m, −1, skill, level, v):
/// 64
/// client creates at U's position with record flags 3 (| 4 when v ≠ 0),
/// owner U, target offsets i = 0…63 of the ring (;
/// the client's tables hold the same
/// values); class 176 toggles flag 0x4000 before each create.
fn ring(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    m: u32,
    skill: u16,
    level: i32,
    v: i32,
) -> Result<(), HandlerError> {
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(());
    };
    let mut rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE,
        owner: Some(key),
        class: m,
        x: i32::from(x),
        y: i32::from(y),
        skill: i32::from(skill),
        level,
        ..CreateRecord::default()
    };
    if v != 0 {
        rec.flags |= flag::VELOCITY;
        rec.velocity = v;
    }
    for i in 0..64 {
        (rec.tx, rec.ty) = d2_sim::skills::use_::bodies::ring_offset(i);
        if m == RING_TOGGLE_CLASS {
            rec.flags ^= flag::NO_LIGHT;
        }
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    }
    Ok(())
}

/// The lob with (U, class, 0, 0, tx, ty, skill, level, 0):
/// U of type player or monster; a zero target coordinate takes
/// [`cast_target_point`], still zero → none. Record flags 0x521 (start
/// position + absolute target + arc + frames from the distance), owner
/// U, start U's position, arc height 30. The result counts as made.
pub(crate) fn lob(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    class: u32,
    (mut tx, mut ty): (i32, i32),
    skill: u16,
    level: i32,
) -> Result<bool, HandlerError> {
    if key.unit_type > MONSTER {
        return Ok(false);
    }
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(false);
    };
    if tx == 0 || ty == 0 {
        (tx, ty) = cast_target_point(w, key);
        if tx == 0 || ty == 0 {
            return Ok(false);
        }
    }
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE | flag::ARC | flag::FRAMES_FROM_DISTANCE,
        owner: Some(key),
        class,
        x: i32::from(x),
        y: i32::from(y),
        tx,
        ty,
        arc_height: 30,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    Ok(create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?.is_some())
}

/// The client start function 19 (Teeth) with (U, skill, level): m :=
/// `cltmissilec`, outside the missile table → 0. T := [`cast_target_point`],
/// (dx, dy) := T − U, d := max + ⌊min / 2⌋ of |dx|, |dy|,
/// at least 1. One client create (record flags 3: start position +
/// relative target; owner U) at U + (2·dx / d, 2·dy / d) toward the
/// offset (5·dx, 5·dy); a missile made faces T (from U's
/// path position, written) and the start returns 1,
/// else 0.
pub fn bone_cast(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<bool, HandlerError> {
    let Some(row) = inputs.tables.skills.get(usize::from(skill)) else {
        return Ok(false);
    };
    let m = i32::from(row.cltmissile_abc[2]);
    if m < 0 || m as usize >= inputs.tables.missiles.len() {
        return Ok(false);
    }
    let Some((ux, uy)) = w.units.get(&key).map(|u| u.cell()) else {
        return Ok(false);
    };
    let (ux, uy) = (i32::from(ux), i32::from(uy));
    let (tx, ty) = cast_target_point(w, key);
    let (dx, dy) = (tx - ux, ty - uy);
    let (ax, ay) = (dx.abs(), dy.abs());
    let d = (ax.max(ay) + ax.min(ay) / 2).max(1);
    let rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_RELATIVE,
        owner: Some(key),
        class: m as u32,
        x: ux + 2 * dx / d,
        y: uy + 2 * dy / d,
        tx: 5 * dx,
        ty: 5 * dy,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    let Some(made) = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)? else {
        return Ok(false);
    };
    let dir = match (u16::try_from(tx), u16::try_from(ty)) {
        (Ok(x), Ok(y)) => super::critter_path::direction_to(w, key, x, y),
        _ => None,
    };
    if let (Some(dir), Some(mis)) = (dir, w.objclient.missiles.get_mut(&made)) {
        mis.direction = dir;
    }
    Ok(true)
}

/// The spray with (U, record, s, n):
/// from the record's
/// start (x, y) and a corner q := U's seed step & 3, n creates: the
/// corner (±16, ±16) of q, q := q + 1 (mod 4), the start := (x, y) + a
/// roll of 2s − s per axis, the target := start + corner, then the
/// callback: the missile's direction := q · 16. Returns the
/// last callback argument (q · 16), which the record keeps for later
/// creates (function 32).
fn spray(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    rec: &mut CreateRecord,
    s: i32,
    n: i32,
) -> Result<u8, HandlerError> {
    const CORNERS: [(i32, i32); 4] = [(-16, -16), (-16, 16), (16, 16), (16, -16)];
    let mut q = unit_step(w, key) & 3;
    let (x, y) = (rec.x, rec.y);
    for _ in 0..n {
        let (cx, cy) = CORNERS[q as usize];
        q = (q + 1) & 3;
        rec.x = x + unit_roll(w, key, 2 * s) as i32 - s;
        rec.y = y + unit_roll(w, key, 2 * s) as i32 - s;
        (rec.tx, rec.ty) = (rec.x + cx, rec.y + cy);
        if let Some(k) = create(w, &inputs.tables.missiles, rec, inputs.high_light_quality)? {
            if let Some(m) = w.objclient.missiles.get_mut(&k) {
                m.direction = ((q << 4) & 63) as u8;
            }
        }
    }
    Ok(((q << 4) & 63) as u8)
}

#[cfg(test)]
mod tests {
    use super::test_util::*;

    #[test]
    fn the_do_tail_locks_the_local_player_for_min_delay_12_updates() {
        use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
        fn blank<T: Record>() -> T {
            T::decode(&vec![0u8; T::SIZE])
        }
        let run = |delay: u8, anim: u8, event: u32, local: bool| {
            let (mut w, key) = world();
            w.drlg_updates = 100;
            if local {
                w.local_player = Some(key);
            }
            w.units.get_mut(&key).unwrap().event_index = event;
            let mut i = inputs(0);
            i.tables.skills[usize::from(FROST_NOVA)].anim = anim;
            let mut skills = vec![blank::<Skills>(); 50];
            // `delay` at offset 0 of `skillscode`: the constant.
            skills[usize::from(FROST_NOVA)].delay = 0;
            i.skill_tables = Some(Arc::new(d2_sim::skills::SkillTables {
                skills,
                skilldesc: vec![blank::<Skilldesc>()],
                missiles: vec![blank::<Missiles>()],
                skills_code: vec![0x07, delay, 0x00],
                miss_code: Vec::new(),
                level_cap: 99,
                stat_count: 359,
            }));
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
            w.skill_lock
        };
        assert_eq!(run(50, 10, 0, true), 112);
        assert_eq!(run(5, 10, 0, true), 105);
        assert_eq!(run(0, 10, 0, true), 0);
        assert_eq!(run(50, 10, 0, false), 0);
        // A sequence skill (anim 0x12) locks only at event index 0.
        assert_eq!(run(50, 0x12, 0, true), 112);
        assert_eq!(run(50, 0x12, 1, true), 0);
    }

    #[test]
    fn the_rewind_keeps_100_minus_p_percent_of_the_whole_frames() {
        // Strafe: 1944 (frame 7) with Param6 50 →
        // 768, then the advance (243) → 1011.
        let mut f = 1944;
        rewind(&mut f, 50);
        assert_eq!(f, 768);
        let mut f = 0x7FF;
        rewind(&mut f, 100);
        assert_eq!(f, 0);
        // Fend: 2479 (frame 9) with Fend's
        // Param2 60 → 768 (+281 → 1049).
        let mut f = 2479;
        rewind(&mut f, 60);
        assert_eq!(f, 768);
    }

    #[test]
    fn a_null_entry_only_sets_flag_40_and_bad_ids_do_nothing() {
        let (mut w, key) = world();
        assert_eq!(
            client_do(&mut w, &inputs(0), key, FROST_NOVA, 3).unwrap(),
            1
        );
        assert!(w.units[&key].flag_40 && missiles(&w).is_empty());
        let (mut w, key) = world();
        for f in [-1, 130] {
            assert_eq!(
                client_do(&mut w, &inputs(f), key, FROST_NOVA, 3).unwrap(),
                0
            );
        }
        // A skill outside the table.
        assert_eq!(client_do(&mut w, &inputs(25), key, 50, 3).unwrap(), 0);
        assert!(!w.units[&key].flag_40 && missiles(&w).is_empty());
        // A missile class outside the table: 0, flag untouched.
        let mut i = inputs(25);
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc[0] = 9;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 0);
        assert!(!w.units[&key].flag_40);
    }

    #[test]
    fn the_tail_requests_dosound_on_u_and_tgtsound_on_the_target_unit() {
        use crate::bridge::client_missiles::MissileSound;
        let mut i = inputs(0);
        let row = &mut i.tables.skills[usize::from(FROST_NOVA)];
        (row.dosound, row.tgtsound) = (40, 41);
        // No target unit: only dosound.
        let (mut w, key) = world();
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        assert_eq!(
            w.objclient.missile_sounds,
            [MissileSound::Request {
                id: 40,
                missile: key
            }]
        );
        // A unit target: tgtsound on it, after dosound; a point target
        // has no unit.
        let t = UnitKey::new(MONSTER, 9);
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Unit(t));
        client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap();
        assert_eq!(
            w.objclient.missile_sounds,
            [
                MissileSound::Request {
                    id: 40,
                    missile: key
                },
                MissileSound::Request { id: 41, missile: t },
            ]
        );
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Point(1, 2));
        client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap();
        assert_eq!(w.objclient.missile_sounds.len(), 1);
        // A do that did not run (bad entry): no tail.
        let mut j = i.clone();
        j.tables.skills[usize::from(FROST_NOVA)].cltdofunc = -1;
        let (mut w, key) = world();
        client_do(&mut w, &j, key, FROST_NOVA, 3).unwrap();
        assert!(w.objclient.missile_sounds.is_empty());
    }

    #[test]
    fn the_player_update_runs_the_do_once_on_the_update_after_the_event_frame() {
        let (mut w, key) = world();
        w.local_player = Some(key);
        let mut i = inputs(25);
        i.player_anims = Some(Arc::new(Cast));
        // The do's tail reads the skill delay (the use lock); no skill rows → 0.
        i.skill_tables = Some(Arc::new(d2_sim::skills::SkillTables {
            skills: Vec::new(),
            skilldesc: Vec::new(),
            missiles: Vec::new(),
            skills_code: Vec::new(),
            miss_code: Vec::new(),
            level_cap: 99,
            stat_count: 359,
        }));
        player_anim::mode_set(&mut w, &i, key, 10);
        // Update 1 advances 0 → 1, update 2 crosses frame 2 (+0x4E := 2),
        // update 3 runs the do before its advance.
        for n in 1..=2 {
            player_anim::step(&mut w, &i, key).unwrap();
            assert!(missiles(&w).is_empty(), "update {n}");
        }
        assert_eq!(w.units[&key].action_frame, 2);
        player_anim::step(&mut w, &i, key).unwrap();
        assert_eq!(missiles(&w).len(), 64);
        assert!(w.units[&key].flag_40);
        // Flag 0x40 holds the rest of the mode: no second ring; the end
        // clears the used skill and the neutral mode set clears the flag.
        for _ in 0..3 {
            player_anim::step(&mut w, &i, key).unwrap();
        }
        let u = &w.units[&key];
        assert_eq!((u.mode, missiles(&w).len()), (1, 64));
        assert!(!u.flag_40 && u.skills.as_ref().unwrap().current.is_none());
    }

    #[test]
    fn a_remote_player_without_an_action_frame_does_the_skill_at_the_mode_end() {
        let (mut w, key) = world();
        let i = ModelInputs {
            player_anims: None,
            ..inputs(25)
        };
        // No record: count 0, complete on the first update; not the
        // local player, flag 0x40 clear → the do at the end.
        player_anim::mode_set(&mut w, &i, key, 10);
        player_anim::step(&mut w, &i, key).unwrap();
        assert_eq!((w.units[&key].mode, missiles(&w).len()), (1, 64));
    }

    #[test]
    fn the_monster_skill_step_runs_the_do_on_its_event_byte_once_per_mode() {
        use crate::bridge::monster_anim::{mode_set, skill_step};
        let (mut w, i, key) = caster((5010, 5004));
        skill_step(&mut w, &i, key).unwrap();
        assert!(missiles(&w).is_empty(), "event byte 0: no do");
        event(&mut w, key, 2);
        skill_step(&mut w, &i, key).unwrap();
        assert_eq!(missiles(&w).len(), 1);
        assert!(w.units[&key].flag_40);
        // The `cltmissile` aims at the path's target point.
        assert_eq!(missiles(&w)[0].target_point, (5010, 5004));
        // Flag 0x40 holds bytes 1–3 ...
        skill_step(&mut w, &i, key).unwrap();
        assert_eq!(missiles(&w).len(), 1);
        // ... not byte 4.
        event(&mut w, key, 4);
        skill_step(&mut w, &i, key).unwrap();
        assert_eq!(missiles(&w).len(), 2);
        // A mode set clears the flag, even to the same mode.
        let mode = w.units[&key].mode;
        mode_set(&mut w, &i, key, mode);
        assert!(!w.units[&key].flag_40);
        event(&mut w, key, 1);
        skill_step(&mut w, &i, key).unwrap();
        assert_eq!(missiles(&w).len(), 3);
        // No used skill, or not a monster: nothing.
        w.units.get_mut(&key).unwrap().used_skill = None;
        event(&mut w, key, 4);
        skill_step(&mut w, &i, key).unwrap();
        assert_eq!(missiles(&w).len(), 3);
    }

    #[test]
    fn a_monster_cast_without_a_target_point_aims_one_step_ahead() {
        // Path direction 16 = octant 2: (−2, 0).
        let (mut w, i, key) = caster((0, 0));
        event(&mut w, key, 4);
        crate::bridge::monster_anim::skill_step(&mut w, &i, key).unwrap();
        assert_eq!(missiles(&w)[0].target_point, (4998, 5000));
    }

    #[test]
    fn the_monster_start_48_makes_cltmissilea_at_the_caster() {
        let mut i = inputs(74);
        i.tables.skills[usize::from(FROST_NOVA)].cltstfunc = 48;
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(monster_start(&mut w, &i, key, FROST_NOVA, 4).unwrap(), 1);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].level, 4);
        assert_eq!(monster_start(&mut w, &i, key, 9999, 4).unwrap(), 0);
        // Another start function: nothing.
        i.tables.skills[usize::from(FROST_NOVA)].cltstfunc = 11;
        let (mut w, key) = aimed_at((5006, 4997));
        assert_eq!(monster_start(&mut w, &i, key, FROST_NOVA, 4).unwrap(), 1);
        assert!(missiles(&w).is_empty());
    }

    #[test]
    fn a_lob_row_throws_its_cltmissile_in_an_arc() {
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Point(5012, 5000));
        let mut i = inputs(0);
        let row = &mut i.tables.skills[usize::from(FROST_NOVA)];
        (row.cltmissile, row.lob) = (CLASS as i16, true);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 3).unwrap(), 1);
        let ms = missiles(&w);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].target_point, (5012, 5000));
        // Frames from the distance: 12 << 16 / (v << 4), v = 9 · 256 · 3 / 4.
        let v = ((9 << 8) * 75 / 100) as u32;
        assert_eq!(ms[0].total, ((12u32 << 16) / (v << 4)) as i32);
        assert_eq!(ms[0].direction, ((5012 + 5000) & 63) as u8);
    }

    #[test]
    fn the_teeth_start_makes_one_cast_missile_ahead_of_the_caster() {
        let (mut w, key) = world();
        w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Point(5010, 5004));
        let mut i = inputs(17);
        i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [-1, -1, CLASS as i16];
        assert!(bone_cast(&mut w, &i, key, FROST_NOVA, 3).unwrap());
        // (dx, dy) = (10, 4), d = 10 + 2 = 12: start U + (20 / 12, 8 / 12)
        // = (5001, 5000), target start + (50, 20).
        assert_eq!(
            w.objclient
                .set_c
                .values()
                .map(|u| u.position)
                .collect::<Vec<_>>(),
            [Some((5001, 5000))]
        );
        let ms = missiles(&w);
        assert_eq!(ms[0].target_point, (5051, 5020));
        assert_eq!(
            Some(ms[0].direction),
            crate::bridge::critter_path::direction_to(&w, key, 5010, 5004)
        );
        // No `cltmissilec`: no missile, the start fails.
        let (mut w, key) = world();
        assert!(!bone_cast(&mut w, &inputs(17), key, FROST_NOVA, 3).unwrap());
        assert!(missiles(&w).is_empty());
    }
}
