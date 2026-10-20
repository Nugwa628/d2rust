//! The client's use state of the local player's click (over the
//! model) and the client start's `cltstfunc` result (step 5), which
//! decide whether a skill click sends anything (local player step 2:
//! the C→S message leaves only when the mode request returned
//! non-zero).

mod s006_talon_start;
mod s008_s015_inferno_start;
mod s013_strafe_start;
mod s014_s053_reach_start;
mod s020_s021_corpse_start;
mod s025_charge_start;
mod s030_leap_start;
mod s031_whirlwind_start;
mod s032_magott_up_start;
mod s042_mosquito_start;

use super::dispatch::HandlerError;
use super::items::ItemView;
use super::msg::skills::level_with_bonuses;
use super::skills::{SkillEntry, NATIVE};
use super::world::{
    CastTarget, ClientWorld, KindData, ModelInputs, SkillRow, UnitKey, ITEM, MONSTER,
};
use crate::controls::click::skill_flag;

/// The use state codes.
pub mod code {
    pub const USABLE: u32 = 0;
    pub const NO_MANA: u32 = 1;
    pub const SHAPE: u32 = 4;
    pub const DISABLED: u32 = 3;
    pub const NO_ITEM: u32 = 2;
    pub const PASSIVE: u32 = 5;
    pub const AURA: u32 = 6;
    pub const NO_LEVEL: u32 = 7;
    pub const COOLDOWN: u32 = 8;
}

/// State 121 `skilldelay`.
const STATE_SKILL_DELAY: u8 = 121;

/// State 114 `blood_mana`.
const STATE_BLOOD_MANA: u8 = 114;
/// The were-forms' `srvdofunc`.
const SRVDOFUNC_WERE: i16 = 116;
/// Stats `hitpoints` (6) and `mana` (8).
const STAT_LIFE: u16 = 6;
const STAT_MANA: u16 = 8;
/// Skills 0 (Attack), 4 (Left Hand Throw), 5 (Left Hand Swing) of the
/// item type test.
const SKILL_ATTACK: u16 = 0;
const SKILL_LEFT_THROW: u16 = 4;
const SKILL_LEFT_SWING: u16 = 5;
/// Stats `item_magicarrow` (157) and `quantity` (70) of the ammo rule.
const STAT_MAGIC_ARROW: u16 = 157;
const STAT_QUANTITY: u16 = 70;
/// Stat 204 `item_charged_skill` (the charges test 9).
const STAT_CHARGED_SKILL: u16 = 204;
/// Flag-ex (+0xC8) bit 0x8: the shapeshift test
/// (DT start step 1).
const FLAG_EX_SHAPESHIFT: u32 = 0x8;

/// `use_state(P, entry)` (first failure wins) of
/// `key`'s entry `e`: (1) no record or `InGame` clear → 3; (2) level 0
/// → 7; (3) `aura` → 6; (4) `passive` → 5; (5) the item type test → 2;
/// (6) `can_afford` fails → 1; (7) shape → 4;
/// (8) the start stat → 1 (Blade Fury with mana 1024, cost 864,
/// `startmana` 6 << 8, sends nothing); (9) the charges → 2.
// (10) a player with state 121 and a `delay` formula → 8 (the Molten
// Boulder clicks 13 and 25 ticks after a cast, `Delay` 50, send nothing).
// Test 10 reads a `delay` calc as > 0: exact for the base-game `skills.txt`,
// whose 23 non-empty `delay` cells are all plain integers > 0 (data
// check, Constants), so no calc evaluator is needed.
// PROVISIONAL: the local player's code
// 8 of the client lock reads as passing: the model holds no client lock;
// the client lock would need its own model.
pub fn use_state(w: &ClientWorld, inputs: &ModelInputs, key: UnitKey, e: &SkillEntry) -> u32 {
    let t = &inputs.tables;
    let Some(r) = t.skills.get(usize::from(e.skill)) else {
        return code::DISABLED;
    };
    if !r.ingame {
        return code::DISABLED;
    }
    let level = level_with_bonuses(w, key, &t.skills, &t.skilldesc, e);
    if level == 0 {
        return code::NO_LEVEL;
    }
    if r.aura {
        return code::AURA;
    }
    if r.flags & crate::controls::click::skill_flag::PASSIVE != 0 {
        return code::PASSIVE;
    }
    if !item_type_test(w, key, e.skill, r) {
        return code::NO_ITEM;
    }
    if !can_afford(w, key, r, e, level) {
        return code::NO_MANA;
    }
    if !shape_ok(w, inputs, key, r) {
        return code::SHAPE;
    }
    if !start_stat_ok(w, key, r) {
        return code::NO_MANA;
    }
    if !charges_ok(w, e) {
        return code::NO_ITEM;
    }
    let delayed = w.units.get(&key).is_some_and(|u| {
        key.unit_type == super::world::PLAYER && u.states.contains(&STATE_SKILL_DELAY)
    });
    if delayed && r.delay.is_some() {
        return code::COOLDOWN;
    }
    code::USABLE
}

/// The item type test (test 5): the skill's sets a / b against the items worn at body
/// locations 4 (A) and 5 (B) of the local player's inventory; for skill
/// 4 / 5 the weapon in use leaves the test (step 3); a matched item then
/// meets the item flag and `shoots` ammo rules of `hand`
/// (a bow and no quiver sends no
/// C→S 0x0C / 0x0D at the click). Without the item tables the test
/// passes.
fn item_type_test(w: &ClientWorld, key: UnitKey, skill: u16, r: &SkillRow) -> bool {
    use super::attack_weapon::{weapon_in_use, worn_at};
    let Some(tables) = w.item_tables.0.clone() else {
        return true;
    };
    let [ia, ib] = r.itypes;
    let [ea, eb] = r.etypes;
    // Step 1.
    if ea[0] <= 0 && ia[0] <= 0 {
        return true;
    }
    let (mut ha, mut hb) = (worn_at(w, key, 4), worn_at(w, key, 5));
    // Step 3: Left Hand Throw / Swing drop the weapon in use.
    if matches!(skill, SKILL_LEFT_THROW | SKILL_LEFT_SWING) {
        if let Some(u) = weapon_in_use(w, key).map(|v| v.key) {
            if ha.as_ref().is_some_and(|a| a.key == u) {
                ha = None;
            } else if hb.as_ref().is_some_and(|b| b.key == u) {
                hb = None;
            }
        }
    }
    let listed = |t: &[i16]| t.iter().copied().take_while(|&t| t > 0).collect::<Vec<_>>();
    let item_is = |v: &ItemView, t: i16| v.code.is_some_and(|c| tables.item_is_type(c, t));
    let noammo = r.flags & skill_flag::NO_AMMO != 0;
    let matched = |x: &ItemView, y: Option<&ItemView>| {
        matched_item_ok(w, skill, noammo, x.key, y.map(|y| y.key), |k, t| {
            super::items::item(w, k).is_some_and(|v| item_is(&v, t))
        })
    };
    // `hand(s, X, Y)`.
    let hand = |it: &[i16], et: &[i16], x: Option<&ItemView>, y: Option<&ItemView>| match x {
        None => {
            if et[0] <= 0 && it[0] <= 0 {
                return true;
            }
            [45, 46, 67].contains(&ia[0]) && ib[0] <= 0 && !y.is_some_and(|y| item_is(y, 45))
        }
        Some(x) => {
            if listed(et).into_iter().any(|t| item_is(x, t)) {
                return false;
            }
            // No type listed passes without the matched item's rules.
            let want = listed(it);
            if want.is_empty() {
                return true;
            }
            want.into_iter().any(|t| item_is(x, t)) && matched(x, y)
        }
    };
    let (a, b) = (ha.as_ref(), hb.as_ref());
    // Step 4.
    if hand(&ia, &ea, a, b) {
        hand(&ib, &eb, b, a)
    } else {
        hand(&ib, &eb, a, b) && hand(&ia, &ea, b, a)
    }
}

/// The matched item's rules of `hand`:
/// X (`x`, matched by set s) with item flag 0x4000 or 0x100 (the
/// stream's F and the bits the model writes) passes only for Attack;
/// else when X's primary type `shoots` an ammo type s and the skill lacks
/// `noammo`: Attack and X with stat 157 passes; Y (`y`, the other hand)
/// none or not of type s fails; Attack and Y with stat 157 passes; a
/// stackable Y with quantity (70) < 1 fails. `is_type` is the caller's
/// type test. Without the item tables only the flag rule
/// acts. Also read by the character panel (`ui::char_desc`).
pub(crate) fn matched_item_ok(
    w: &ClientWorld,
    skill: u16,
    noammo: bool,
    x: UnitKey,
    y: Option<UnitKey>,
    is_type: impl Fn(UnitKey, i16) -> bool,
) -> bool {
    use super::attack_weapon::item_total;
    let Some(xv) = super::items::item(w, x) else {
        return true;
    };
    let model = match w.units.get(&x).map(|u| &u.kind) {
        Some(KindData::Item(d)) => d.flags,
        _ => 0,
    };
    if (xv.flags | model) & (0x4000 | 0x100) != 0 {
        return skill == SKILL_ATTACK;
    }
    let Some(tables) = w.item_tables.0.clone() else {
        return true;
    };
    let s = xv.code.map_or(0, |c| tables.type_shoots(c));
    if s == 0 || noammo {
        return true;
    }
    if skill == SKILL_ATTACK && item_total(w, x, STAT_MAGIC_ARROW) != 0 {
        return true;
    }
    let Some(y) = y.filter(|&y| is_type(y, s)) else {
        return false;
    };
    if skill == SKILL_ATTACK && item_total(w, y, STAT_MAGIC_ARROW) != 0 {
        return true;
    }
    let stackable = super::items::item(w, y)
        .and_then(|v| v.code)
        .is_some_and(|c| tables.stackable(c));
    !(stackable && item_total(w, y, STAT_QUANTITY) < 1)
}

/// The start stat test (test 8): a player
/// with a `startmana` > 0 skill fails while its mana (stat 8) is below
/// `startmana` << 8, unless it has state 12 (`inferno`, the channel
/// already running).
fn start_stat_ok(w: &ClientWorld, key: UnitKey, r: &SkillRow) -> bool {
    let start = i32::from(r.startmana as i16);
    if key.unit_type != super::world::PLAYER || start <= 0 {
        return true;
    }
    w.total(key, STAT_MANA, 0) >= start << 8
        || w.units
            .get(&key)
            .is_some_and(|u| u.states.contains(&STATE_INFERNO))
}

/// The charges test (test 9): an item
/// skill (owner GUID ≠ −1) passes while the owner item's
/// `item_charged_skill` (stat 204) at layer (base level & 0x3F) + skill ·
/// 64 has a non-zero low byte (the charges left); an owner item the model
/// does not hold passes.
fn charges_ok(w: &ClientWorld, e: &SkillEntry) -> bool {
    if e.owner == NATIVE {
        return true;
    }
    let item = UnitKey::new(ITEM, e.owner);
    let Some(KindData::Item(d)) = w.units.get(&item).map(|u| &u.kind) else {
        return true;
    };
    let layer = ((e.base & 0x3F) as u16).wrapping_add(e.skill.wrapping_mul(64));
    let v = d
        .props
        .iter()
        .chain(d.fields.iter().flat_map(|f| f.base.iter()))
        .filter(|p| p.stat == STAT_CHARGED_SKILL && p.layer == layer)
        .fold(0i32, |a, p| a.wrapping_add(p.value));
    v as u8 != 0
}

/// The shape test (test 7): `restrict` 0
/// passes only outside a shapeshift form (no state with the `states.txt`
/// `restrict` flag), 1 always, 2 only in a form that has
/// one of the row's `State1`…`State3` (a negative id ends the list).
fn shape_ok(w: &ClientWorld, inputs: &ModelInputs, key: UnitKey, r: &SkillRow) -> bool {
    let Some(u) = w.units.get(&key) else {
        return false;
    };
    let in_form = || {
        u.states.iter().any(|&s| {
            inputs
                .tables
                .states
                .get(usize::from(s))
                .is_some_and(|row| row.restrict)
        })
    };
    match r.restrict {
        0 => !in_form(),
        2 => {
            in_form()
                && r.shape_states
                    .iter()
                    .take_while(|&&s| s >= 0)
                    .any(|&s| u.states.contains(&(s as u8)))
        }
        _ => true,
    }
}

/// `can_afford(unit, skill)`: a
/// charge entry needs charges; else c = (mana + lvlmana · max(L − 1, 0))
/// << manashift against life (state 114) or mana; a were-form skill while
/// shapeshifted always passes.
fn can_afford(w: &ClientWorld, key: UnitKey, r: &SkillRow, e: &SkillEntry, level: i32) -> bool {
    if e.owner != NATIVE {
        return e.charges > 0;
    }
    let n = (level - 1).max(0);
    let c = i32::from(r.mana as i16)
        .wrapping_add(i32::from(r.lvlmana as i16).wrapping_mul(n))
        .wrapping_shl(u32::from(r.manashift as u8));
    let Some(u) = w.units.get(&key) else {
        return false;
    };
    if u.states.contains(&STATE_BLOOD_MANA) {
        w.total(key, STAT_LIFE, 0) >= c
    } else if r.srvdofunc == SRVDOFUNC_WERE && u.flag_ex & FLAG_EX_SHAPESHIFT != 0 {
        true
    } else {
        w.total(key, STAT_MANA, 0) >= c
    }
}

/// The `cltstfunc` result of the client skill start
/// (step 5) for skill `skill` with target `target` (the unit of a
/// code 0x16 request; none for 0x15, whose start clears the target):
/// false = the start returns 0 (current := none, mode set 1, and the
/// click sends nothing). The bodies of the table are;
/// on the click U has its used entry and the skill
/// is valid, so the functions that test only those pass.
// PROVISIONAL: the tests the model holds no
// facts for read as passing: the ammo test (1, 2, 11, 13;
// test 5's item type test covers a missing quiver), the pet test (3),
// the row's state (7), the range calc (16), the target point's town room
// (17, 22), the items row bit and the item test of the golem item (23),
// `monstats2` flags 8 / 12 and uniques (24, 26, 51), the
// client missile creations (19, 24, 43, 46, 47) and the path / spot
// searches (25, 29–32, 35, 36); settled by porting the bodies of those
// functions to the model.
pub fn client_start_passes(
    w: &ClientWorld,
    inputs: &ModelInputs,
    caster: UnitKey,
    skill: u16,
    target: Option<UnitKey>,
) -> bool {
    let Some(r) = inputs.tables.skills.get(usize::from(skill)) else {
        return true;
    };
    let unit = target.and_then(|k| w.units.get(&k));
    match r.cltstfunc {
        // A target T, U's
        // and T's rooms not in town, T a player or monster
        // and hostile.
        5 => unit.is_some_and(|u| {
            u.key.unit_type <= MONSTER
                && !super::modes::in_town(w, caster)
                && !super::modes::in_town(w, u.key)
                && super::combat::hostile(w, inputs, u.key)
        }),
        // (Raise): U's used
        // entry E, a target T and the raise test with (T, 0).
        20 => {
            s020_s021_corpse_start::has_entry(w, caster)
                && unit.is_some_and(|u| s020_s021_corpse_start::raise_corpse(inputs, u, false))
        }
        // (the explosions):
        // E and T; the corpse test with (T) only when U is the local
        // player.
        21 => {
            s020_s021_corpse_start::has_entry(w, caster)
                && unit.is_some_and(|u| {
                    w.local_player != Some(caster)
                        || s020_s021_corpse_start::raise_corpse(inputs, u, true)
                })
        }
        24 => unit.is_some_and(|u| u.key.unit_type == MONSTER && u.is_dead()),
        // An item on the ground (mode 3), identified (flag
        // 0x10).
        23 => target
            .and_then(|k| super::items::item(w, k))
            .is_some_and(|v| v.mode == super::items::mode::GROUND && v.flags & 0x10 != 0),
        // T a player or monster.
        10 => unit.is_some_and(|u| u.key.unit_type <= MONSTER),
        // (Find Item): the corpse test with (T).
        28 => unit.is_some_and(|u| s020_s021_corpse_start::raise_corpse(inputs, u, true)),
        // A target.
        34 | 39 | 42 | 50 => unit.is_some(),
        _ => true,
    }
}

/// The state `cltstfunc` 15 (Inferno)
/// works on: 12.
pub const STATE_INFERNO: u8 = 12;

/// Stat 68 `attackrate`, set by [`attach_attack_rate`].
pub const STAT_ATTACKRATE: u16 = 68;

/// The client's attack-rate list with (U, v) (the client twin of
/// the server's): a new list
/// (pool 0, flags 4 TEMPONLY, expire 0, owner U) attached to U with
/// `attackrate` (68) := v, then U's rate. The list goes at
/// U's next real mode change. Callers:
/// `cltstfunc` 9 (`Param4`) and 27 (`eval(cltcalc1)`).
pub fn attach_attack_rate(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, v: i32) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    u.temp_lists
        .push([((STAT_ATTACKRATE, 0), v)].into_iter().collect());
    super::player_anim::refresh_rate(w, inputs, key);
}

/// The side effects of a passing `cltstfunc` (step 5), run right after [`client_start_passes`] with the start's
/// level L (the local player's level with
/// bonuses, else the request's).
///
/// Function 9 (Dragon Tail): [`attach_attack_rate`] with
/// the row's `Param4` (+0x154).
///
/// Function 15 (Inferno): without a state-12 list on U the
/// client makes one (flag 2 NEWLENGTH, expire 21 client updates; its
/// removal callback clears state 12) and sets state 12;
/// with one, the sequence position := (position & 0xFF) | 0x800, the
/// frame count +0x48 := 0x500 and the list's expire := 7. The overlay
/// (+0x10C) and sound (+0x10E) of the first branch are blank for every
/// row using function 15 and are not modelled.
///
/// Function 6 (Dragon Talon)
/// writes the used skill entry's +0x18: 0 without a
/// target unit, else the skill's `calc1` (+0x138)
/// at the start's level L (with bonuses): the
/// kicks its `cltdofunc` 4 counts down.
///
/// Function 8 (Blade Fury) is the
/// same on state 12 but for: the new list expires after 20 updates and
/// the used entry's parameter 1 := 0; with a list the
/// position := (position & 0xFF) + (`seqinput` − 1) · 256 and the frame
/// count := 0x800.
///
/// Function 18 (the curses):
/// `cltmissilec` (+0xEE) inside the missiles table →
/// the missile create with (U, cltmissilec, S, L).
///
/// Function 13 (Strafe) writes the arrow count and
/// the first target into E +0x18 / +0x1C / +0x20 ([`s013_strafe_start::strafe_start`]).
/// Functions 14 (Fend) and 53 (Zeal, Fury)
/// do the same for the melee hits ([`s014_s053_reach_start::reach_start`]).
///
/// Functions 20 (Raise) and 21 (the
/// explosions) write the target's
/// position, and 21 its class, into U's used entry
/// (`s020_s021_corpse_start::corpse_start`).
///
/// Function 27 (Double Swing):
/// v := `cltcalc1` (+0x114); the stat list create with (U, v)
/// attaches a TEMPONLY list (flags 4) to U with stat 68 := v, then the
/// rate refresh with (U).
pub fn client_start_apply(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    caster: UnitKey,
    skill: u16,
    level: i32,
) -> Result<(), HandlerError> {
    let Some(r) = inputs.tables.skills.get(usize::from(skill)) else {
        return Ok(());
    };
    match r.cltstfunc {
        6 => s006_talon_start::talon_start(w, inputs, caster, skill),
        25 => s025_charge_start::charge_start(w, caster),
        20 | 21 => {
            let t = crate::bridge::client_do::path_target_unit(w, caster);
            s020_s021_corpse_start::corpse_start(w, caster, t, r.cltstfunc == 21);
        }
        30 => s030_leap_start::leap_start(w, inputs, caster, skill),
        31 => s031_whirlwind_start::whirlwind_start(w, inputs, caster),
        32 => s032_magott_up_start::magott_up_start(w, inputs, caster, skill, level)?,
        42 => s042_mosquito_start::mosquito_start(w, inputs, caster, skill, level)?,
        13 => s013_strafe_start::strafe_start(w, inputs, caster, skill),
        14 | 53 => {
            let _ = s014_s053_reach_start::reach_start(w, inputs, caster, skill, r.cltstfunc == 14);
        }
        9 => attach_attack_rate(w, inputs, caster, r.param4),
        15 => s008_s015_inferno_start::inferno_start(w, caster, false, 0),
        8 => s008_s015_inferno_start::inferno_start(w, caster, true, i32::from(r.seqinput)),
        18 => {
            let c = i32::from(r.cltmissile_abc[2]);
            if usize::try_from(c).is_ok_and(|i| i < inputs.tables.missiles.len()) {
                super::client_do::cast_toward(w, inputs, caster, c, skill, level)?;
            }
        }
        27 => {
            let v = super::client_do::cltcalc1(w, inputs, caster, skill, level)?;
            attach_attack_rate(w, inputs, caster, v);
        }
        _ => {}
    }
    Ok(())
}

/// The melee range test with (U, T) for a start's target unit: `None` where the
/// melee range is not modelled (a caster other than the local player, no
/// item tables), and the start then changes nothing (its hit start
/// is not modelled either).
fn target_in_reach(
    w: &ClientWorld,
    inputs: &ModelInputs,
    caster: UnitKey,
    t: UnitKey,
) -> Option<bool> {
    let env = super::client_do::search_env(inputs);
    super::client_missiles::in_melee_range(w, &env, caster, t, 0).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::skills::SkillList;
    use crate::bridge::world::{ClientUnit, PLAYER};

    fn world(mana: i32) -> (ClientWorld, ModelInputs, UnitKey) {
        let mut inputs = ModelInputs::default();
        // Row 85 BloodGolem-like: mana 25, lvlmana 4, shift 8; row 70
        // Raise Skeleton-like: mana 6, lvlmana 1.
        inputs.tables.skills = vec![SkillRow::default(); 100];
        inputs.tables.skills[85] = SkillRow {
            ingame: true,
            mana: 25,
            lvlmana: 4,
            manashift: 8,
            ..SkillRow::default()
        };
        inputs.tables.skills[70] = SkillRow {
            ingame: true,
            mana: 6,
            lvlmana: 1,
            manashift: 8,
            cltstfunc: 20,
            ..SkillRow::default()
        };
        let key = UnitKey::new(PLAYER, 1);
        let mut p = ClientUnit::new(key);
        p.stats.insert(STAT_MANA, mana);
        p.skills = Some(SkillList::default());
        let mut w = ClientWorld::default();
        w.units.insert(key, p);
        (w, inputs, key)
    }

    const PLAYER_KEY: UnitKey = UnitKey::new(PLAYER, 1);

    /// Dragon Tail's start (`cltstfunc` 9, `Param4` -40) adds -40 to U's
    /// `attackrate` until U's next mode change (TEMPONLY), as
    /// the base-game kick (mode 12) runs at speed 153, 60%.
    #[test]
    fn dragon_tail_start_slows_until_the_mode_changes() {
        let (mut w, mut inputs, key) = world(100);
        inputs.tables.skills.resize(271, SkillRow::default());
        inputs.tables.skills[270] = SkillRow {
            ingame: true,
            cltstfunc: 9,
            param4: -40,
            ..SkillRow::default()
        };
        super::super::player_anim::mode_set(&mut w, &inputs, key, 12);
        client_start_apply(&mut w, &inputs, key, 270, 1).unwrap();
        assert_eq!(w.total(key, STAT_ATTACKRATE, 0), -40);
        client_start_apply(&mut w, &inputs, key, 270, 1).unwrap();
        assert_eq!(w.total(key, STAT_ATTACKRATE, 0), -80);
        super::super::player_anim::mode_set(&mut w, &inputs, key, 1);
        assert_eq!(w.total(key, STAT_ATTACKRATE, 0), 0);
        assert!(w.units[&key].temp_lists.is_empty());
    }

    fn entry(skill: u16, base: i32) -> SkillEntry {
        SkillEntry {
            skill,
            base,
            owner: NATIVE,
            ..SkillEntry::default()
        }
    }

    /// Molten Boulder (`Delay` 50) cast
    /// at tick 264; the clicks at 277 and 289 send nothing (state 121 on).
    #[test]
    fn a_delay_skill_under_state_121_is_use_state_8() {
        let (mut w, mut i, k) = world(25856);
        i.tables.skills[70].delay = Some(0x40);
        assert_eq!(use_state(&w, &i, k, &entry(70, 20)), code::USABLE);
        w.units
            .get_mut(&k)
            .unwrap()
            .states
            .insert(STATE_SKILL_DELAY);
        assert_eq!(use_state(&w, &i, k, &entry(70, 20)), code::COOLDOWN);
        // A skill without a delay formula is not held by the cooldown.
        assert_eq!(use_state(&w, &i, k, &entry(85, 20)), code::USABLE);
    }

    /// Blood Golem: level 20 costs (25 + 4 ·
    /// 19) << 8 = 25856; the necromancer's 21248 mana refuses it (no C→S
    /// at the click), Raise Skeleton's (6 + 19) << 8 = 6400 passes.
    #[test]
    fn mana_below_the_level_cost_is_use_state_1() {
        let (w, i, k) = world(21248);
        assert_eq!(use_state(&w, &i, k, &entry(85, 20)), code::NO_MANA);
        assert_eq!(use_state(&w, &i, k, &entry(70, 20)), code::USABLE);
        let (w, i, k) = world(25856);
        assert_eq!(use_state(&w, &i, k, &entry(85, 20)), code::USABLE);
        // Level 0 and a record without `InGame` come first.
        assert_eq!(use_state(&w, &i, k, &entry(85, 0)), code::NO_LEVEL);
        assert_eq!(use_state(&w, &i, k, &entry(3, 1)), code::DISABLED);
        assert_eq!(use_state(&w, &i, k, &entry(500, 1)), code::DISABLED);
    }

    #[test]
    fn a_corpse_start_needs_the_entry_and_a_raisable_corpse() {
        use crate::bridge::skills::SkillEntry;
        use crate::bridge::world::{MonsterClass, MonsterSetup, StateRow};
        let (mut w, mut i, _) = world(0);
        i.tables.skills[71] = SkillRow {
            cltstfunc: 21,
            ..SkillRow::default()
        };
        let fallen = MonsterClass {
            corpse_sel: true,
            setup: Some(MonsterSetup {
                velocity: 6,
                ..MonsterSetup::default()
            }),
            ..MonsterClass::default()
        };
        i.tables.monsters = vec![None, Some(fallen)];
        i.tables.states = vec![
            StateRow::default(),
            StateRow {
                udead: true,
                ..StateRow::default()
            },
        ];
        let m = UnitKey::new(MONSTER, 9);
        let mut c = ClientUnit::new(m);
        (c.class, c.mode) = (1, 12);
        w.units.insert(m, c);
        // No used entry: 0 for both.
        assert!(!client_start_passes(&w, &i, PLAYER_KEY, 70, Some(m)));
        assert!(!client_start_passes(&w, &i, PLAYER_KEY, 71, Some(m)));
        w.units.get_mut(&PLAYER_KEY).expect("p").skills = Some(SkillList {
            entries: vec![SkillEntry::default()],
            current: Some(0),
            ..SkillList::default()
        });
        assert!(!client_start_passes(&w, &i, PLAYER_KEY, 70, None));
        assert!(client_start_passes(&w, &i, PLAYER_KEY, 70, Some(m)));
        assert!(client_start_passes(&w, &i, PLAYER_KEY, 71, Some(m)));
        // Mode 0 (dying) is not raisable; a `udead` state fails both tests.
        w.units.get_mut(&m).expect("m").mode = 0;
        assert!(!client_start_passes(&w, &i, PLAYER_KEY, 70, Some(m)));
        w.units.get_mut(&m).expect("m").mode = 12;
        w.units.get_mut(&m).expect("m").states.insert(1);
        assert!(!client_start_passes(&w, &i, PLAYER_KEY, 70, Some(m)));
        // Function 21 tests the corpse only for the local player.
        assert!(client_start_passes(&w, &i, PLAYER_KEY, 71, Some(m)));
        w.local_player = Some(PLAYER_KEY);
        assert!(!client_start_passes(&w, &i, PLAYER_KEY, 71, Some(m)));
        w.units.get_mut(&m).expect("m").states.clear();
        assert!(client_start_passes(&w, &i, PLAYER_KEY, 71, Some(m)));
        // Velocity 0: no raise, the corpse test still passes.
        i.tables.monsters[1] = Some(MonsterClass {
            setup: None,
            ..fallen
        });
        assert!(!client_start_passes(&w, &i, PLAYER_KEY, 70, Some(m)));
        assert!(client_start_passes(&w, &i, PLAYER_KEY, 71, Some(m)));
        // A function without a rule passes.
        assert!(client_start_passes(&w, &i, PLAYER_KEY, 85, None));
    }

    #[test]
    fn the_explosion_start_writes_the_corpse_point_and_class() {
        use crate::bridge::skills::SkillEntry;
        use crate::bridge::world::CastTarget;
        let (mut w, mut i, _) = world(0);
        i.tables.skills[71] = SkillRow {
            cltstfunc: 21,
            ..SkillRow::default()
        };
        let m = UnitKey::new(MONSTER, 9);
        let mut c = ClientUnit::new(m);
        (c.class, c.position) = (19, Some((4000, 4100)));
        w.units.insert(m, c);
        let p = w.units.get_mut(&PLAYER_KEY).expect("p");
        p.skills = Some(SkillList {
            entries: vec![SkillEntry::default()],
            current: Some(0),
            ..SkillList::default()
        });
        p.cast_target = Some(CastTarget::Unit(m));
        client_start_apply(&mut w, &i, PLAYER_KEY, 71, 1).unwrap();
        let e = w.units[&PLAYER_KEY].skills.as_ref().expect("l").entries[0];
        assert_eq!((e.param, e.param2, e.counter), (4000, 4100, 19));
        // Function 20 leaves +0x20.
        let p = w.units.get_mut(&PLAYER_KEY).expect("p");
        p.skills.as_mut().expect("l").entries[0] = SkillEntry::default();
        client_start_apply(&mut w, &i, PLAYER_KEY, 70, 1).unwrap();
        let e = w.units[&PLAYER_KEY].skills.as_ref().expect("l").entries[0];
        assert_eq!((e.param, e.param2, e.counter), (4000, 4100, 0));
    }

    /// Shock Wave (`restrict` 2, `State1`
    /// bear) is refused outside bear form; `restrict` 0 refuses in a form.
    #[test]
    fn shape_restriction_is_use_state_4() {
        let (mut w, mut i, k) = world(100_000);
        i.tables.states = vec![Default::default(); 150];
        i.tables.states[140].restrict = true; // a form
        i.tables.states[141].restrict = true; // the bear
        i.tables.skills[85].restrict = 2;
        i.tables.skills[85].shape_states = [141, -1, 0];
        i.tables.skills[70].restrict = 0;
        assert_eq!(use_state(&w, &i, k, &entry(85, 20)), code::SHAPE);
        assert_eq!(use_state(&w, &i, k, &entry(70, 20)), code::USABLE);
        // In another form: still refused; the list stops at a negative id.
        w.units.get_mut(&k).expect("p").states.insert(140);
        assert_eq!(use_state(&w, &i, k, &entry(85, 20)), code::SHAPE);
        assert_eq!(use_state(&w, &i, k, &entry(70, 20)), code::SHAPE);
        w.units.get_mut(&k).expect("p").states.insert(141);
        assert_eq!(use_state(&w, &i, k, &entry(85, 20)), code::USABLE);
        // `restrict` 1: any form.
        i.tables.skills[85].restrict = 1;
        assert_eq!(use_state(&w, &i, k, &entry(85, 20)), code::USABLE);
    }

    /// Item tables of the ammo rule: `sbw ` (short bow) is of 45 `weap`
    /// and 27 `bow`, whose `shoots` is 5 `bowq`; `aqv ` (arrows) is of
    /// 5 `bowq` and stackable.
    struct BowTables;
    impl crate::bridge::item_lists::StreamProps for BowTables {
        fn props(&self, _: &[u8]) -> (Vec<crate::bridge::item_lists::ItemProp>, bool) {
            (Vec::new(), false)
        }
        fn item_is_type(&self, code: [u8; 4], t: i16) -> bool {
            match &code {
                b"sbw " => t == 45 || t == 27,
                b"aqv " => t == 5,
                _ => false,
            }
        }
        fn type_shoots(&self, code: [u8; 4]) -> i16 {
            if &code == b"sbw " {
                5
            } else {
                0
            }
        }
        fn stackable(&self, code: [u8; 4]) -> bool {
            &code == b"aqv "
        }
    }

    /// An identified item stream head at body
    /// location `loc` (mode 1).
    fn body_stream(loc: u8, code: &[u8; 4]) -> Vec<u8> {
        let bits: [(u32, u32); 8] = [
            (0x10, 32),
            (0x65, 10),
            (1, 3),
            (u32::from(loc), 4),
            (0, 4),
            (0, 4),
            (0, 3),
            (u32::from_le_bytes(*code), 32),
        ];
        let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
        for (v, w) in bits {
            acc |= u64::from(v) << n;
            n += w;
            while n >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            out.push(acc as u8);
        }
        out
    }

    fn wear(w: &mut ClientWorld, guid: u32, loc: u8, code: &[u8; 4], quantity: i32) {
        use crate::bridge::item_lists::ItemProp;
        use crate::bridge::world::{ItemData, ItemRecord};
        let ik = UnitKey::new(ITEM, guid);
        let mut i = ClientUnit::new(ik);
        i.kind = KindData::Item(ItemData {
            last: Some(ItemRecord {
                id: 0x9C,
                action: 0x04,
                category: 0,
                owner: None,
                seq: 0,
                stream: body_stream(loc, code),
            }),
            props: vec![ItemProp {
                stat: STAT_QUANTITY,
                layer: 0,
                value: quantity,
            }],
            ..Default::default()
        });
        w.units.insert(ik, i);
    }

    /// Multiple Shot
    /// (`itypea1` 27 `bow`) with a short bow and no quiver sends no C→S
    /// 0x0C / 0x0D at the click (use state 2); with arrows in hand 5 it
    /// casts.
    #[test]
    fn a_bow_without_ammo_fails_the_item_type_test() {
        let (mut w, mut i, k) = world(1000);
        w.local_player = Some(k);
        w.item_tables =
            crate::bridge::item_lists::ItemTablesRef(Some(std::sync::Arc::new(BowTables)));
        i.tables.skills[12] = SkillRow {
            ingame: true,
            itypes: [[27, 0, 0], [0, 0, 0]],
            ..SkillRow::default()
        };
        // No hand item: the bow set is not a weapon-type set (step 4).
        assert_eq!(use_state(&w, &i, k, &entry(12, 20)), code::NO_ITEM);
        wear(&mut w, 9, 4, b"sbw ", 0);
        assert_eq!(use_state(&w, &i, k, &entry(12, 20)), code::NO_ITEM);
        // `noammo` lifts the ammo rule.
        i.tables.skills[12].flags = skill_flag::NO_AMMO;
        assert_eq!(use_state(&w, &i, k, &entry(12, 20)), code::USABLE);
        i.tables.skills[12].flags = 0;
        // An empty stack of arrows fails; one arrow passes.
        wear(&mut w, 10, 5, b"aqv ", 0);
        assert_eq!(use_state(&w, &i, k, &entry(12, 20)), code::NO_ITEM);
        wear(&mut w, 10, 5, b"aqv ", 1);
        assert_eq!(use_state(&w, &i, k, &entry(12, 20)), code::USABLE);
        // A broken bow (flag 0x100) passes only for Attack.
        if let Some(KindData::Item(d)) =
            w.units.get_mut(&UnitKey::new(ITEM, 9)).map(|u| &mut u.kind)
        {
            d.flags |= 0x100;
        }
        assert_eq!(use_state(&w, &i, k, &entry(12, 20)), code::NO_ITEM);
    }

    /// Blade Fury
    /// (mana 8, lvlmana 1, shift 5, `startmana` 6) at level 20 costs 864;
    /// mana 1024 passes test 6 and fails test 8 (< 1536): code 1, no cast.
    /// In state 12 (`inferno`) the start stat is not asked.
    #[test]
    fn mana_below_the_start_stat_is_use_state_1() {
        let (mut w, mut i, k) = world(1024);
        i.tables.skills.resize(267, SkillRow::default());
        i.tables.skills[266] = SkillRow {
            ingame: true,
            mana: 8,
            lvlmana: 1,
            manashift: 5,
            startmana: 6,
            ..SkillRow::default()
        };
        assert_eq!(use_state(&w, &i, k, &entry(266, 20)), code::NO_MANA);
        w.units.get_mut(&k).unwrap().states.insert(STATE_INFERNO);
        assert_eq!(use_state(&w, &i, k, &entry(266, 20)), code::USABLE);
        let (w, i2, k) = world(1536);
        let mut i2 = i2;
        i2.tables.skills = i.tables.skills.clone();
        assert_eq!(use_state(&w, &i2, k, &entry(266, 20)), code::USABLE);
    }

    /// Functions 34, 39, 42, 50 need a target unit; 10 a player or
    /// monster one; 4 never fails.
    #[test]
    fn client_starts_that_need_a_target() {
        let (mut w, mut i, k) = world(100);
        let m = UnitKey::new(MONSTER, 9);
        w.units.insert(m, ClientUnit::new(m));
        let it = UnitKey::new(ITEM, 10);
        w.units.insert(it, ClientUnit::new(it));
        for (skill, f) in [(1u16, 39u16), (2, 10), (4, 4)] {
            i.tables.skills[usize::from(skill)].cltstfunc = f;
        }
        assert!(!client_start_passes(&w, &i, k, 1, None));
        assert!(client_start_passes(&w, &i, k, 1, Some(m)));
        assert!(!client_start_passes(&w, &i, k, 2, Some(it)));
        assert!(client_start_passes(&w, &i, k, 2, Some(m)));
        assert!(client_start_passes(&w, &i, k, 4, None));
    }
}
