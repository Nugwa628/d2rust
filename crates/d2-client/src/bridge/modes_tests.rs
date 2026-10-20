//! Tests of the client mode machines.

use super::dispatch::HandlerError;
use super::drlg::DrlgRoomId;
use super::modes::{
    mode_request, monster_table_mode, player_mode as pm, FATAL_OBJECT_CODE, FATAL_PLAYER_CODE,
};
use super::objects::{ObjSound, ObjUnit, NO_LOCAL_DISTANCE};
use super::output::{Output, Outputs};
use super::overlay_lights::OverlayLightRow;
use super::world::{
    ActiveRoom, ClientUnit, ClientWorld, ModelInputs, MonsterClass, ObjectRow, UnitKey, ITEM,
    MISSILE, MONSTER, OBJECT, PLAYER,
};

const P: UnitKey = UnitKey::new(PLAYER, 1);

fn world_with(key: UnitKey, mode: u32) -> ClientWorld {
    let mut w = ClientWorld::default();
    let mut u = ClientUnit::new(key);
    u.mode = mode;
    u.position = Some((50, 50));
    w.units.insert(key, u);
    w
}

fn req(
    w: &mut ClientWorld,
    i: &ModelInputs,
    key: UnitKey,
    code: u8,
    r: [i32; 7],
) -> Result<Vec<Output>, HandlerError> {
    let out = Outputs::default();
    mode_request(w, i, key, code, r, &out)?;
    Ok(out.take())
}

const R0: [i32; 7] = [0, 0, 0x2A, 0, 0, 0, 0];

#[test]
fn player_codes_set_the_table_modes() {
    let i = ModelInputs::default();
    for (code, mode) in [
        (0x00, pm::WALK),
        (0x01, pm::WALK),
        (0x06, pm::GET_HIT),
        (0x07, pm::NEUTRAL),
        (0x08, pm::DEATH),
        (0x09, pm::DEAD),
        (0x14, pm::SEQUENCE),
        (0x17, pm::RUN),
        (0x18, pm::RUN),
        (0x19, pm::BLOCK),
    ] {
        let mut w = world_with(P, 1);
        w.units.get_mut(&P).unwrap().mode = 7;
        req(&mut w, &i, P, code, R0).unwrap();
        let u = &w.units[&P];
        assert_eq!(u.mode, mode, "code {code:#x}");
        assert_eq!(u.last_mode_request.map(|m| m.code), Some(code));
    }
    // Codes that keep the mode: 0x02, 0x12 (no inventory input), 0x13,
    // 0x15, 0x16.
    for code in [0x02, 0x12, 0x13, 0x15, 0x16] {
        let mut w = world_with(P, 7);
        req(&mut w, &i, P, code, R0).unwrap();
        assert_eq!(w.units[&P].mode, 7, "code {code:#x}");
    }
}

#[test]
fn player_hit_class_flag_and_path_stop() {
    let i = ModelInputs::default();
    for code in [0x06, 0x08, 0x12, 0x13, 0x14] {
        let mut w = world_with(P, 1);
        req(&mut w, &i, P, code, R0).unwrap();
        assert_eq!(w.units[&P].hit_class, 0x2A, "code {code:#x}");
    }
    let mut w = world_with(P, 1);
    req(&mut w, &i, P, 0x07, R0).unwrap();
    assert_eq!(w.units[&P].flag_2, Some(true));
    // A unit in mode 0x13 with flag 1: its path is stopped first.
    let mut w = world_with(P, pm::SEQUENCE);
    req(&mut w, &i, P, 0x17, R0).unwrap();
    assert!(w.units[&P].path_stopped);
    let mut w = world_with(P, 1);
    req(&mut w, &i, P, 0x17, R0).unwrap();
    assert!(!w.units[&P].path_stopped);
}

/// Code 7 on a dead player places it at (PROVISIONAL:
/// the respawn's 0x0D, after the 0x15 the dead player ignored); a living
/// player stays where it is, and a dead one with record (0, 0) too.
#[test]
fn code_7_places_a_player_that_was_dead() {
    let i = ModelInputs::default();
    let back = [103, 23, 0, 0, 0, 0, 0];
    for dead in [pm::DEAD, pm::DEATH] {
        let mut w = world_with(P, dead);
        req(&mut w, &i, P, 0x07, back).unwrap();
        assert_eq!(w.units[&P].position, Some((103, 23)), "mode {dead}");
        assert_eq!(w.units[&P].mode, pm::NEUTRAL);
    }
    let mut w = world_with(P, pm::NEUTRAL);
    req(&mut w, &i, P, 0x07, back).unwrap();
    assert_eq!(w.units[&P].position, Some((50, 50)));
    let mut w = world_with(P, pm::DEAD);
    req(&mut w, &i, P, 0x07, R0).unwrap();
    assert_eq!(w.units[&P].position, Some((50, 50)));
}

#[test]
fn player_invalid_codes_are_fatal_0x432() {
    let i = ModelInputs::default();
    for code in [3u8, 4, 5, 0x0A, 0x0F, 0x11, 0x1A, 0xFF] {
        let mut w = world_with(P, 1);
        assert_eq!(
            req(&mut w, &i, P, code, R0),
            Err(HandlerError::Fatal(FATAL_PLAYER_CODE)),
            "code {code:#x}"
        );
    }
}

#[test]
fn town_walk_and_neutral() {
    let i = ModelInputs::default();
    let room = DrlgRoomId(3);
    for (level, walk, neutral) in [
        (1u16, pm::TOWN_WALK, pm::TOWN_NEUTRAL),
        (2, pm::WALK, pm::NEUTRAL),
    ] {
        let mut w = world_with(P, 1);
        w.active_rooms = Some(vec![ActiveRoom {
            x0: 0,
            y0: 0,
            w: 100,
            h: 100,
            level,
            room,
        }]);
        w.room_units.place(P, Some(room));
        req(&mut w, &i, P, 0x00, R0).unwrap();
        assert_eq!(w.units[&P].mode, walk);
        req(&mut w, &i, P, 0x07, R0).unwrap();
        assert_eq!(w.units[&P].mode, neutral);
    }
}

fn object_inputs() -> ModelInputs {
    let mut i = ModelInputs::default();
    let row = ObjectRow {
        lit: [0, 0, 8, 0, 0, 0, 0, 0],
        rgb: (255, 128, 0),
        ..ObjectRow::default()
    };
    i.tables.objects = vec![row; 4];
    i
}

#[test]
fn object_code_3_mode_light_and_sound() {
    let i = object_inputs();
    let o = UnitKey::new(OBJECT, 9);
    let mut w = world_with(o, 0);
    w.units.get_mut(&o).unwrap().class = 2;
    let out = req(&mut w, &i, o, 3, [1, 2, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&o].mode, 2);
    assert_eq!(w.lights.len(), 1);
    assert_eq!(
        out,
        [Output::ObjectSound(ObjSound::Mode {
            unit: ObjUnit {
                key: o,
                client_only: false
            },
            class: 2,
            mode: 2,
            local_dist: NO_LOCAL_DISTANCE,
        })]
    );
    // Back to a mode with `Lit` 0: the light is removed.
    req(&mut w, &i, o, 3, [1, 0, 0, 0, 0, 0, 0]).unwrap();
    assert!(w.lights.is_empty());
    // Code 0x15 changes nothing here (the shrine use is the handler's);
    // any other code is fatal 0x39C.
    assert!(req(&mut w, &i, o, 0x15, R0).unwrap().is_empty());
    assert_eq!(
        req(&mut w, &i, o, 2, R0),
        Err(HandlerError::Fatal(FATAL_OBJECT_CODE))
    );
}

#[test]
fn item_code_2_and_missiles() {
    let i = ModelInputs::default();
    let it = UnitKey::new(ITEM, 4);
    let mut w = world_with(it, 0);
    req(&mut w, &i, it, 2, [1, 3, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!((w.units[&it].mode, w.units[&it].flag_2), (3, Some(true)));
    req(&mut w, &i, it, 2, [0, 5, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!((w.units[&it].mode, w.units[&it].flag_2), (5, Some(false)));
    req(&mut w, &i, it, 7, [0, 1, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&it].mode, 5);
    // A missile's request returns at once and is not stored.
    let m = UnitKey::new(MISSILE, 2);
    let mut w = world_with(m, 0);
    req(&mut w, &i, m, 1, R0).unwrap();
    assert_eq!(w.units[&m].last_mode_request, None);
}

/// Inputs whose `monstats` has class 0 with a `monstats2` row (the
/// monster class gate).
fn monster_inputs() -> ModelInputs {
    let mut i = ModelInputs::default();
    // A class with the walk mode (`monstats2` `mWL`, the code 7 test).
    i.tables.monsters = vec![Some(MonsterClass {
        modes: 1 << 2,
        ..MonsterClass::default()
    })];
    i
}

/// The mode a monster in `mode` ends in after the request `code`.
fn monster_after(i: &ModelInputs, mode: u32, code: u8, r: [i32; 7]) -> u32 {
    let k = UnitKey::new(MONSTER, 5);
    let mut w = world_with(k, mode);
    req(&mut w, i, k, code, r).unwrap();
    w.units[&k].mode
}

/// +0xB0 := for 0x06 and 0x14, for 0x13 (no mode change):
/// for the death 0x08.
#[test]
fn monster_hit_codes_store_the_hit_class() {
    let i = monster_inputs();
    let k = UnitKey::new(MONSTER, 5);
    for (code, want) in [(0x06, 6), (0x08, 6), (0x13, 1), (0x14, 6)] {
        let mut w = world_with(k, 1);
        req(&mut w, &i, k, code, [1, 2, 3, 4, 5, 0, 6]).unwrap();
        assert_eq!(w.units[&k].hit_class, want, "code {code:#x}");
    }
    assert_eq!(monster_after(&i, 1, 0x13, [1, 2, 3, 4, 5, 0, 6]), 1);
}

/// Codes 0x06 (with or
/// without the spray) and 0x13 arm the hit flash counter (U +0x58 := 3);
/// 0x14 and 0x08 do not; each unit draw takes one, down to 0.
#[test]
fn hit_codes_arm_the_hit_flash_and_unit_draws_take_it() {
    let i = monster_inputs();
    let k = UnitKey::new(MONSTER, 5);
    for (code, want) in [(0x06, 3), (0x13, 3), (0x14, 0), (0x08, 0)] {
        let mut w = world_with(k, 1);
        req(&mut w, &i, k, code, [1, 2, 3, 4, 5, 0, 6]).unwrap();
        assert_eq!(w.units[&k].hit_flash, want, "code {code:#x}");
    }
    let mut w = world_with(k, 1);
    req(&mut w, &i, k, 0x06, [1, 2, 3, 4, 5, 0, 6]).unwrap();
    let left: Vec<u8> = (0..4)
        .map(|_| {
            w.take_hit_flashes(&[k]);
            w.units[&k].hit_flash
        })
        .collect();
    assert_eq!(left, [2, 1, 0, 0]);
}

/// row 0x14: ≠ 0 → stat 6 := << 8. The code 6 that ends
/// the knockback then reads an equal life byte and does not spray, so
/// U's seed is not drawn (the spray's GH gate would draw once).
/// Measured on the base game: a bashed Fallen 1:30, 0x14 = 64 then
/// 6 = 63 (L = 64), no blood.
#[test]
fn knockback_life_stops_the_get_hit_spray() {
    let i = monster_inputs();
    let k = UnitKey::new(MONSTER, 5);
    let run = |r5: i32| {
        let mut w = world_with(k, 2);
        let u = w.units.get_mut(&k).unwrap();
        u.stats.insert(6, 128 << 8);
        u.seed = Some((130_316_480, 410_823_396));
        req(&mut w, &i, k, 0x14, [51, 50, 5, 11, 75, r5, 113]).unwrap();
        let after_kb = w.units[&k].stat(6);
        req(&mut w, &i, k, 0x06, [51, 50, 63, 2, 0, 4, 113]).unwrap();
        (after_kb, w.units[&k].stat(6), w.units[&k].seed)
    };
    let (kb, gh, seed) = run(64);
    assert_eq!((kb, gh), (64 << 8, 64 << 8));
    assert_eq!(seed, Some((130_316_480, 410_823_396)), "no spray, no draw");
    // = 0 leaves stat 6: the life byte differs and the gate draws.
    let (kb, gh, seed) = run(0);
    assert_eq!((kb, gh), (128 << 8, 64 << 8));
    assert_ne!(seed, Some((130_316_480, 410_823_396)));
}

/// Death (0x08) stores in +0xB0 too, for the death sound's
/// impact pair (a fire kill is 0x2D).
#[test]
fn monster_death_stores_the_hit_class() {
    let i = monster_inputs();
    let k = UnitKey::new(MONSTER, 5);
    let mut w = world_with(k, 1);
    req(&mut w, &i, k, 0x08, [1, 2, 3, 4, 5, 0, 0x2D]).unwrap();
    assert_eq!(w.units[&k].hit_class, 0x2D);
    assert_eq!(w.units[&k].mode, 0);
}

#[test]
fn the_neutral_fallback_stops_the_path_at_the_cell_centre() {
    let i = monster_inputs();
    let k = UnitKey::new(MONSTER, 5);
    let mut w = world_with(k, 2);
    w.units.get_mut(&k).unwrap().precise = Some(((50 << 16) | 32392, (50 << 16) | 33144));
    req(&mut w, &i, k, 0x07, [51, 50, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&k].mode, 1);
    assert_eq!(
        w.units[&k].precise,
        Some(((50 << 16) | 0x8000, (50 << 16) | 0x8000))
    );
}

#[test]
fn monster_mode_follows_the_dispatch_table() {
    let i = monster_inputs();
    // 7: walk after the position check (far from the target).
    assert_eq!(monster_after(&i, 1, 0x07, R0), 2);
    // 7 within 1 sub-tile of the target: F.
    assert_eq!(monster_after(&i, 2, 0x07, [51, 50, 0, 0, 0, 0, 0]), 1);
    // 7 for a class without `mWL` (its mode-2 test is 0): F.
    let mut no_walk = monster_inputs();
    no_walk.tables.monsters = vec![Some(MonsterClass::default())];
    assert_eq!(monster_after(&no_walk, 2, 0x07, R0), 1);
    // 2, 3, 0x19 and an unknown code: F (mode 1 from 1..15 except 12).
    assert_eq!(monster_after(&i, 2, 0x02, R0), 1);
    assert_eq!(monster_after(&i, 12, 0x02, R0), 12);
    assert_eq!(monster_after(&i, 0, 0x03, R0), 0);
    assert_eq!(monster_after(&i, 4, 0x19, R0), 1);
    assert_eq!(monster_after(&i, 4, 0x1E, R0), 1);
    // 0x13: no mode change.
    assert_eq!(monster_after(&i, 4, 0x13, R0), 4);
    assert_eq!(monster_after(&i, 1, 0x08, R0), 0);
    assert_eq!(monster_after(&i, 1, 0x06, R0), 3);
    assert_eq!(monster_after(&i, 1, 0x12, R0), 6);
    assert_eq!(monster_after(&i, 1, 0x14, R0), 13);
    assert_eq!(monster_after(&i, 1, 0x01, R0), 2);
    assert_eq!(monster_after(&i, 1, 0x17, R0), 15);
    // 0 / 0x18: path to a unit; the unit absent → F.
    assert_eq!(monster_after(&i, 4, 0x00, [0, 77, 0, 0, 0, 0, 0]), 1);
    assert_eq!(monster_after(&i, 4, 0x18, [0, 5, 0, 0, 0, 0, 0]), 1);
    assert_eq!(
        monster_after(&i, 4, 0x00, [MONSTER as i32, 5, 0, 0, 0, 0, 0]),
        2
    );
    assert_eq!(
        monster_after(&i, 4, 0x18, [MONSTER as i32, 5, 0, 0, 0, 0, 0]),
        15
    );
    // The T table of the point and unit groups.
    for (code, mode) in [
        (0x04, 7),
        (0x05, 7),
        (0x0A, 4),
        (0x0B, 4),
        (0x0C, 8),
        (0x0D, 8),
        (0x0E, 9),
        (0x0F, 9),
        (0x10, 5),
        (0x11, 5),
        (0x1A, 10),
        (0x1B, 10),
        (0x1C, 11),
        (0x1D, 11),
    ] {
        assert_eq!(monster_table_mode(code), Some(mode));
        assert_eq!(monster_after(&i, 1, code, R0), mode, "{code:#x}");
    }
    assert_eq!(monster_table_mode(0x07), None);
    // Code 9 kills.
    let k = UnitKey::new(MONSTER, 5);
    let mut w = world_with(k, 1);
    req(&mut w, &i, k, 9, R0).unwrap();
    assert!(w.units[&k].is_dead());
    req(&mut w, &i, k, 2, R0).unwrap();
    assert_eq!(w.units[&k].mode, 12);
}

#[test]
fn monster_code_7_in_state_attached_falls_back() {
    let i = monster_inputs();
    let k = UnitKey::new(MONSTER, 5);
    let mut w = world_with(k, 2);
    w.units.get_mut(&k).unwrap().states.insert(143);
    req(&mut w, &i, k, 0x07, R0).unwrap();
    assert_eq!(w.units[&k].mode, 1);
}

/// The monster light ends with the death (codes 8 and 9 remove it)
/// and comes back with any other code to the
/// dead monster (from the head).
#[test]
fn a_monster_light_ends_with_its_death_and_returns_with_a_later_code() {
    use super::msg::lighting::{monster_light, unit_light};
    let mut i = monster_inputs();
    let class = i.tables.monsters[0].as_mut().unwrap();
    class.light = 5;
    class.light_rgb = (255, 255, 255);
    let row = *class;
    let k = UnitKey::new(MONSTER, 5);
    let mut w = world_with(k, 1);
    monster_light(&mut w, k, &row);
    assert!(unit_light(&w, k).is_some());
    req(&mut w, &i, k, 0x08, R0).unwrap();
    assert_eq!(w.units[&k].mode, 0);
    assert!(unit_light(&w, k).is_none());
    monster_light(&mut w, k, &row);
    req(&mut w, &i, k, 0x09, R0).unwrap();
    assert_eq!(w.units[&k].mode, 12);
    assert!(unit_light(&w, k).is_none());
    req(&mut w, &i, k, 0x07, R0).unwrap();
    assert!(unit_light(&w, k).is_some());
}

#[test]
fn a_monster_class_without_rows_has_no_mode_switch() {
    let i = ModelInputs::default();
    assert_eq!(monster_after(&i, 1, 0x0A, R0), 1);
    let mut i = ModelInputs::default();
    i.tables.monsters = vec![None];
    assert_eq!(monster_after(&i, 1, 0x0A, R0), 1);
}

/// A monster's 0x15 / 0x16 adds the skill at the request's level and makes
/// it the list's current entry: the used
/// entry the do functions read.
#[test]
fn a_monster_skill_code_makes_the_skill_the_used_entry() {
    use super::skills::{skill_entry, skill_entry_mut, used_entry, used_entry_mut, SkillList};
    use super::world::SkillRow;
    let mut i = monster_inputs();
    i.tables.skills = vec![
        SkillRow::default(),
        SkillRow {
            monanim: 4,
            ..SkillRow::default()
        },
    ];
    let m = UnitKey::new(MONSTER, 2);
    let mut w = world_with(m, 1);
    // Without a list: no entry (the used skill is still recorded).
    req(&mut w, &i, m, 0x15, [1, -1, 0, 0, 7, 0, 0]).unwrap();
    assert!(used_entry(&w, m).is_none());
    assert_eq!(w.units[&m].used_skill, Some(1));
    // With a list: the entry at level 7 becomes current.
    let mut w = world_with(m, 1);
    w.units.get_mut(&m).unwrap().skills = Some(SkillList::default());
    req(&mut w, &i, m, 0x15, [1, -1, 0, 0, 7, 0, 0]).unwrap();
    let e = used_entry(&w, m).unwrap();
    assert_eq!((e.skill, e.base), (1, 7));
    assert_eq!(skill_entry(&w, m, 1), Some(e));
    // The next request reuses the entry: its level replaces the base, the
    // rest of the entry stays.
    used_entry_mut(&mut w, m).unwrap().quantity = 3;
    req(&mut w, &i, m, 0x16, [1, -1, 0, 0, 9, 0, 0]).unwrap();
    let e = used_entry(&w, m).unwrap();
    assert_eq!((e.base, e.quantity), (9, 3));
    assert_eq!(w.units[&m].skills.as_ref().unwrap().entries.len(), 1);
    assert!(skill_entry_mut(&mut w, m, 0).is_none());
}

/// Codes 0x15 / 0x16 (S→C 0x4D / 0x4C) set the skill's `anim` (player)
/// or `monanim` (monster) mode; a skill without a row leaves the mode.
/// PROVISIONAL: the attack pick of skill 0 is not modelled; settled by
/// a capture of the attack pick.
#[test]
fn skill_codes_set_the_skill_animation_mode() {
    use super::world::SkillRow;
    let mut i = monster_inputs();
    i.tables.skills = vec![
        SkillRow::default(),
        SkillRow {
            anim: 10,
            monanim: 4,
            ..SkillRow::default()
        },
        SkillRow {
            anim: 10,
            monanim: 14,
            ..SkillRow::default()
        },
    ];
    let m = UnitKey::new(MONSTER, 2);
    // A monster skill of mode 0xE (sequence) sets mode 14; with
    // no sequence for the class, the AnimData count stays.
    let mut w = world_with(m, 1);
    req(&mut w, &i, m, 0x15, [2, -1, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&m].mode, 14);
    assert_eq!(w.units[&m].used_skill, Some(2));
    assert!(w.units[&m].sequence.is_none());
    // 0x16's target (type, GUID) is present: a monster that cannot
    // find it falls back to neutral (`skill_unit_target_*` below).
    let t = UnitKey::new(PLAYER, 7);
    let r = |skill: i32| [skill, -1, i32::from(PLAYER), 7, 0, 0, 0];
    for (key, code, want) in [(P, 0x16, 10), (P, 0x15, 10), (m, 0x16, 4), (m, 0x15, 4)] {
        let mut w = world_with(key, 1);
        w.units.insert(t, ClientUnit::new(t));
        req(&mut w, &i, key, code, r(1)).unwrap();
        assert_eq!(w.units[&key].mode, want, "{key:?} {code:#x}");
        // No row: unchanged.
        let mut w = world_with(key, 1);
        w.units.insert(t, ClientUnit::new(t));
        req(&mut w, &i, key, code, r(9)).unwrap();
        assert_eq!(w.units[&key].mode, 1);
    }
    let mut w = world_with(m, 1);
    req(&mut w, &i, m, 0x16, [1, -1, 1, 77, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&m].mode, 1, "no unit 1/77: neutral");
}

/// A monster's unit-target skill start (code 0x16, S→C
/// 0x4C) makes the target unit its target and faces it; an absent target
/// sends code 7 with a null record, the neutral fallback.
#[test]
fn skill_unit_target_faces_the_target_or_falls_back_to_neutral() {
    use super::world::SkillRow;
    let mut i = monster_inputs();
    i.tables.skills = vec![
        SkillRow::default(),
        SkillRow {
            monanim: 4,
            ..SkillRow::default()
        },
    ];
    let m = UnitKey::new(MONSTER, 2);
    let t = UnitKey::new(PLAYER, 7);
    let mut w = world_with(m, 1);
    let mut tu = ClientUnit::new(t);
    tu.position = Some((50, 30));
    w.units.insert(t, tu);
    req(&mut w, &i, m, 0x16, [1, -1, i32::from(PLAYER), 7, 0, 0, 0]).unwrap();
    let u = &w.units[&m];
    assert_eq!(u.mode, 4);
    assert_eq!(u.faces, 1, "faced the target");
    // Absent target: mode 1.
    let mut w = world_with(m, 1);
    req(&mut w, &i, m, 0x16, [1, -1, i32::from(PLAYER), 7, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&m].mode, 1);
    assert_eq!(w.units[&m].faces, 0);
    // 0x15 faces the point.
    let mut w = world_with(m, 1);
    req(&mut w, &i, m, 0x15, [1, -1, 50, 30, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&m].mode, 4);
    assert_eq!(w.units[&m].faces, 1, "faced the point");
}

/// B025: the local player turns toward a cast target from where it
/// stands (the walk prediction), not from the last point the server gave
/// it, and each turn is counted for the prediction to adopt.
#[test]
fn the_local_player_faces_a_cast_target_from_its_predicted_position() {
    use super::critter_path::face_point;
    let t = d2_sim::path::tables::PathTables::spec().unwrap();
    let mut w = world_with(P, 1);
    w.local_player = Some(P);
    // The model holds (50, 50); the player stands 30 sub-tiles east.
    w.local_predicted = Some(((80 << 16) | 0x8000, (50 << 16) | 0x8000));
    face_point(&mut w, P, 80, 30);
    let want = |from: (u32, u32), to: (u32, u32)| {
        d2_sim::path::walk::geom::direction_vector(&t, from, to).1 & 63
    };
    let centre = |x: u32, y: u32| ((x << 16) | 0x8000, (y << 16) | 0x8000);
    let u = &w.units[&P];
    assert_eq!(
        u.path_turn.map(|(d, _)| d),
        Some(want(centre(80, 50), centre(80, 30))),
        "north of where it stands"
    );
    assert_ne!(
        u.path_turn.map(|(d, _)| d),
        Some(want(centre(50, 50), centre(80, 30))),
        "not from the stale model point"
    );
    assert_eq!(u.faces, 1);
    // The same direction again still counts: the prediction re-adopts it.
    face_point(&mut w, P, 80, 30);
    assert_eq!(w.units[&P].faces, 2);
}

#[test]
fn a_cast_request_is_kept_past_a_later_request_of_the_same_tick() {
    let i = ModelInputs::default();
    let mut w = world_with(P, pm::NEUTRAL);
    let cast = [59, -1, 52, 50, 2, 0, 0];
    req(&mut w, &i, P, 0x15, cast).unwrap();
    req(&mut w, &i, P, 0x13, [5164, 4656, 45, 0, 0, 0, 0]).unwrap();
    let u = &w.units[&P];
    assert_eq!(u.last_mode_request.unwrap().code, 0x13);
    assert_eq!(u.mode_requests, 2);
    let c = u.last_cast.expect("the cast is kept");
    assert_eq!((c.request.code, c.request.record, c.count), (0x15, cast, 1));
    // An object's code 0x15 is the shrine use, no cast.
    let o = UnitKey::new(OBJECT, 7);
    let mut w = world_with(o, 0);
    let _ = req(&mut w, &i, o, 0x15, R0);
    assert_eq!(w.units[&o].last_cast, None);
}

/// A monster with a server path at the centre of (50, 50), facing 0,
/// and a player at (50, 30) (north-west on screen).
fn monster_facing_a_player(mode: u32) -> (ClientWorld, UnitKey) {
    use d2_sim::path::record::DynamicPath;
    let k = UnitKey::new(MONSTER, 5);
    let mut w = world_with(k, mode);
    let mut p = ClientUnit::new(P);
    p.position = Some((50, 30));
    w.units.insert(P, p);
    w.objclient.s_paths.insert(
        k,
        DynamicPath {
            precise_x: (50 << 16) | 0x8000,
            precise_y: (50 << 16) | 0x8000,
            ..DynamicPath::default()
        },
    );
    (w, k)
}

/// The direction from the centre of (50, 50) to the centre of `to`.
fn dir_to(to: (u32, u32)) -> u8 {
    let t = d2_sim::path::tables::PathTables::spec().unwrap();
    let centre = |c: u32| (c << 16) | 0x8000;
    d2_sim::path::walk::geom::direction_vector(
        &t,
        (centre(50), centre(50)),
        (centre(to.0), centre(to.1)),
    )
    .1 & 63
}

/// 0x16's makes the unit (type, GUID) the
/// path target and faces it (measured on the Fallen shaman).
#[test]
fn a_monster_unit_cast_faces_its_target() {
    let i = monster_inputs();
    let (mut w, k) = monster_facing_a_player(1);
    req(&mut w, &i, k, 0x16, [9, -1, 0, 1, 0, 0, 0]).unwrap();
    assert_eq!(
        super::critter_path::target_unit(&w, super::critter_path::Who::server(k)),
        Some(P)
    );
    let p = &w.objclient.s_paths[&k];
    assert_eq!(p.new_direction, dir_to((50, 30)));
    assert_ne!(p.new_direction, 0);
}

/// unit group: faces the target from the path point,
/// not the unit's drawn point (measured: a Fallen
/// 0x29B0 into its cell takes code 0x0F and turns to 7, not 6).
#[test]
fn a_unit_group_request_faces_from_the_path_point() {
    let i = monster_inputs();
    let (mut w, k) = monster_facing_a_player(1);
    w.units.get_mut(&k).unwrap().precise = Some(((50 << 16) | 0x29B0, (50 << 16) | 0x29B0));
    req(&mut w, &i, k, 0x0F, [0, 1, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(w.objclient.s_paths[&k].new_direction, dir_to((50, 30)));
}

/// F: faces the path's target unit before the stop
/// (measured: the Fallen's walk ends in code 7, 58 → 56).
#[test]
fn the_neutral_fallback_faces_the_target_unit() {
    let i = monster_inputs();
    let (mut w, k) = monster_facing_a_player(2);
    super::critter_path::set_target_unit(&mut w, super::critter_path::Who::server(k), Some(P));
    req(&mut w, &i, k, 0x07, [51, 50, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&k].mode, 1);
    assert_eq!(w.objclient.s_paths[&k].new_direction, dir_to((50, 30)));
}

/// A face at the local player in the update pass is redone
/// with the position the walk prediction steps to after it, while the
/// path still holds the direction it set (measured).
#[test]
fn a_face_at_the_local_player_takes_its_fresh_position() {
    use super::critter_path::{face_target, refresh_local_faces, set_target_unit, Who};
    let (mut w, k) = monster_facing_a_player(1);
    w.local_player = Some(P);
    w.local_predicted = Some(((50 << 16) | 0x8000, (30 << 16) | 0x8000));
    set_target_unit(&mut w, Who::server(k), Some(P));
    face_target(&mut w, k);
    assert_eq!(w.objclient.s_paths[&k].new_direction, dir_to((50, 30)));
    // The walk steps the player on to (30, 30): the face is redone.
    w.local_predicted = Some(((30 << 16) | 0x8000, (30 << 16) | 0x8000));
    refresh_local_faces(&mut w);
    assert_eq!(w.objclient.s_paths[&k].new_direction, dir_to((30, 30)));
    assert!(w.local_faces.is_empty());
    // Turned elsewhere since the face: left alone.
    face_target(&mut w, k);
    w.objclient.s_paths.get_mut(&k).unwrap().new_direction = 7;
    w.local_predicted = Some(((50 << 16) | 0x8000, (30 << 16) | 0x8000));
    refresh_local_faces(&mut w);
    assert_eq!(w.objclient.s_paths[&k].new_direction, 7);
}

/// The mode of a monster skill request is the skill
/// entry's +0x08; the monster's own entry holds the `SkNmode` of its
/// set-up, so `ShamanFire` (`monanim` `xx`)
/// is cast in A2 by `fallenshaman`.
#[test]
fn a_monster_skill_request_takes_the_entry_mode() {
    use super::skills::{SkillEntry, SkillList, NATIVE};
    use super::world::SkillRow;
    let mut i = monster_inputs();
    // `monanim` `xx`: no mode of its own.
    i.tables.skills = vec![
        SkillRow::default(),
        SkillRow {
            monanim: 0xFF,
            ..SkillRow::default()
        },
    ];
    let m = UnitKey::new(MONSTER, 2);
    let mut w = world_with(m, 1);
    w.units.get_mut(&m).unwrap().skills = Some(SkillList {
        entries: vec![SkillEntry {
            skill: 1,
            mode: 5,
            owner: NATIVE,
            ..SkillEntry::default()
        }],
        ..SkillList::default()
    });
    req(&mut w, &i, m, 0x15, [1, -1, 50, 30, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&m].mode, 5, "A2 from the entry");
    // Without the entry: `monanim` `xx` sets no mode.
    let mut w = world_with(m, 1);
    req(&mut w, &i, m, 0x15, [1, -1, 50, 30, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&m].mode, 1);
}

/// Inputs with `overlay` rows up to 147, row 81 (`fire_hit`) lit as in
/// the base game (`InitRadius` 3, `Radius` 1).
fn hit_overlay_inputs() -> ModelInputs {
    let mut i = monster_inputs();
    i.tables.overlays = vec![OverlayLightRow::default(); 160];
    i.tables.overlays[81] = OverlayLightRow {
        frames: 8,
        anim_rate: 16,
        init_radius: 3,
        radius: 1,
        rgb: (255, 128, 0),
        ..OverlayLightRow::default()
    };
    i
}

fn hit_id(w: &ClientWorld, k: UnitKey) -> Option<u16> {
    w.units[&k].last_hit_overlay.map(|h| h.id)
}

/// The high nibble of +0xB0 names the overlay, a zero low nibble
/// none; the soft hit 0x13 runs it with player mode 0x13.
#[test]
fn a_player_soft_hit_creates_the_hit_class_overlay() {
    let i = hit_overlay_inputs();
    for (class, want) in [
        (0x11, Some(54)),
        (0x21, Some(81)),
        (0x32, Some(36)),
        (0x41, Some(1)),
        (0x7F, Some(147)),
        (0x81, Some(21)),
        (0x91, Some(113)),
        (0x20, None),
        (0x51, None),
        (0x01, None),
    ] {
        let mut w = world_with(P, pm::NEUTRAL);
        w.server_ticks = 264;
        req(&mut w, &i, P, 0x13, [50, 50, class, 0, 0, 0, 0]).unwrap();
        assert_eq!(hit_id(&w, P), want, "class {class:#x}");
        if let Some(h) = w.units[&P].last_hit_overlay {
            assert_eq!((h.count, h.tick), (1, 264));
        }
        // Only `fire_hit` has a light.
        let lit = w.overlay_lights.iter().any(|o| o.unit == P && o.id == 81);
        assert_eq!(lit, want == Some(81), "class {class:#x}");
    }
}

/// A mode set runs the hit overlays with its new mode (player
/// get-hit 4); the same mode again sets nothing, so creates none.
#[test]
fn a_mode_set_to_get_hit_creates_the_overlay_once() {
    let i = hit_overlay_inputs();
    let mut w = world_with(P, pm::NEUTRAL);
    req(&mut w, &i, P, 0x06, [50, 50, 0x21, 0, 0, 0, 0]).unwrap();
    assert_eq!(hit_id(&w, P), Some(81));
    req(&mut w, &i, P, 0x06, [50, 50, 0x21, 0, 0, 0, 0]).unwrap();
    assert_eq!(w.units[&P].hit_overlays, 1, "same mode: no set");
    // A mode outside {0, 4, 0x13}: none, the hit class notwithstanding.
    let mut w = world_with(P, pm::NEUTRAL);
    w.units.get_mut(&P).unwrap().hit_class = 0x21;
    req(&mut w, &i, P, 0x17, [60, 60, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(hit_id(&w, P), None);
}

/// code 0x13: +0xB0 :=, then the hit overlays with mode 0xD;
/// a `noOvly` monster gets no light.
#[test]
fn a_monster_soft_hit_creates_the_overlay_unless_no_ovly() {
    let mut i = hit_overlay_inputs();
    let k = UnitKey::new(MONSTER, 5);
    let mut w = world_with(k, 1);
    req(&mut w, &i, k, 0x13, [0x21, 0, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(hit_id(&w, k), Some(81));
    assert_eq!(w.units[&k].mode, 1);
    assert!(w.overlay_lights.iter().any(|o| o.unit == k));
    i.tables.monsters = vec![Some(MonsterClass {
        no_ovly: true,
        ..MonsterClass::default()
    })];
    let mut w = world_with(k, 1);
    req(&mut w, &i, k, 0x13, [0x21, 0, 0, 0, 0, 0, 0]).unwrap();
    assert!(w.overlay_lights.is_empty());
}

/// A monster's 0x15 / 0x16 ends, as a player's start, with the cast
/// light of the skill's `cltmissile`:
/// kind 1, radius 1 to target `Light`, the missile's colours, held as the
/// unit's cast light; the unit's own light stays. Measured: the Fallen
/// shaman (g25) has that kind-1
/// light (target 5, 255 178 64) at the list head beside its monster light.
#[test]
fn a_monster_skill_start_makes_the_cast_light() {
    use super::client_missiles::ClientMissileRow;
    use super::world::SkillRow;
    use crate::rules::lighting::records::LightKind;
    let mut i = monster_inputs();
    i.tables.skills = vec![
        SkillRow::default(),
        SkillRow {
            cltmissile: 2,
            ..SkillRow::default()
        },
        SkillRow::default(),
    ];
    i.tables.missiles = vec![ClientMissileRow::default(); 3];
    (i.tables.missiles[2].light, i.tables.missiles[2].rgb) = (5, (255, 178, 64));
    let m = UnitKey::new(MONSTER, 25);
    let mut w = world_with(m, 1);
    req(&mut w, &i, m, 0x15, [1, -1, 50, 30, 0, 0, 0]).unwrap();
    let id = w.cast_lights[&m];
    let l = w.lights.get(id).unwrap();
    assert_eq!((l.owner_guid, l.kind), (25, LightKind::Plain));
    assert_eq!((l.radius, l.target), (8, 40));
    assert_eq!((l.i, l.r, l.g, l.b), (255, 255, 178, 64));
    // The next cast replaces it.
    req(&mut w, &i, m, 0x15, [1, -1, 50, 30, 0, 0, 0]).unwrap();
    assert_ne!(w.cast_lights[&m], id);
    assert!(w.lights.get(id).is_none());
    // A skill without `cltmissile`: no cast light.
    let mut w = world_with(m, 1);
    req(&mut w, &i, m, 0x15, [2, -1, 50, 30, 0, 0, 0]).unwrap();
    assert!(w.cast_lights.is_empty());
}

/// BaseId 190 dying from mode 0xE starts the death at frame
/// 14 (+0x44 := 0xE00, quiet mode set); from another mode, or another
/// BaseId in mode 0xE, the default D0 starts it at frame 0 (measured on
/// a sand maggot).
#[test]
fn base_id_190_dies_from_the_sequence_at_frame_14() {
    let k = UnitKey::new(MONSTER, 5);
    for (base_id, mode, frame) in [(190, 14, 0xE00), (190, 1, 0), (5, 14, 0)] {
        let mut i = ModelInputs::default();
        i.tables.monsters = vec![Some(MonsterClass {
            base_id,
            ..MonsterClass::default()
        })];
        let mut w = world_with(k, mode);
        req(&mut w, &i, k, 0x08, R0).unwrap();
        assert_eq!(w.units[&k].mode, 0, "base {base_id} mode {mode}");
        assert_eq!(w.units[&k].frame, frame, "base {base_id} mode {mode}");
    }
}

/// The inputs of a monster of class `class` with BaseId `base_id`.
fn death_inputs(class: usize, base_id: u16) -> ModelInputs {
    let mut i = ModelInputs::default();
    i.tables.monsters = vec![None; class + 1];
    i.tables.monsters[class] = Some(MonsterClass {
        base_id,
        ..MonsterClass::default()
    });
    i
}

/// BaseId 559 `baalcrabstairs` dies in mode 12 (DD) with
/// flags-ex 0x40000; 570 `baalclone` as 559 after its client missile 614
/// at U; another BaseId takes D0, mode 0. Every branch clears the
/// selectable bit (flags &= ~2).
#[test]
fn the_death_switch_sets_dd_for_559_and_570() {
    use super::client_missiles::ClientMissileRow;
    let k = UnitKey::new(MONSTER, 5);
    for (base_id, mode, flag_ex, missile) in [
        (559, 12, 0x40000, None),
        (570, 12, 0x40000, Some(614)),
        (5, 0, 0, None),
    ] {
        let mut i = death_inputs(0, base_id);
        i.tables.missiles = vec![ClientMissileRow::default(); 615];
        let mut w = world_with(k, 1);
        w.units.get_mut(&k).unwrap().flag_2 = Some(true);
        req(&mut w, &i, k, 0x08, R0).unwrap();
        let u = &w.units[&k];
        assert_eq!(u.mode, mode, "base {base_id}");
        assert_eq!(u.flag_ex & 0x40000, flag_ex, "base {base_id}");
        assert_eq!(u.flag_2, Some(false), "base {base_id}");
        let made: Vec<u32> = w
            .objclient
            .set_c
            .values()
            .filter(|u| u.key.unit_type == MISSILE)
            .map(|u| u.class)
            .collect();
        assert_eq!(
            made,
            missile.into_iter().collect::<Vec<_>>(),
            "base {base_id}"
        );
    }
}

/// A monster dying in a state of the `hide` group gets flag-ex
/// 0x40000 (not drawn), flags &= ~2 and
/// flag 0x20; one in the `shatter` group gets the shatter missiles of
/// (271 + `Height`, then 275; 427, 426 for state 184
/// without state 1) and +0xB0 := (+0xB0 & 0xF) | 0xA0. In the base game
/// `states.txt` 1 `freeze`, 107 `shatter` and 184 `uberminion` are in
/// both groups.
#[test]
fn a_death_in_the_hide_and_shatter_groups_hides_the_body_and_shatters() {
    use super::client_missiles::ClientMissileRow;
    use super::world::StateRow;
    let k = UnitKey::new(MONSTER, 5);
    let both = StateRow {
        hide: true,
        shatter: true,
        ..StateRow::default()
    };
    for (states, hidden, missiles) in [
        (vec![107], true, vec![272, 275]),
        (vec![1], true, vec![272, 275]),
        (vec![184], true, vec![427, 426]),
        (vec![184, 1], true, vec![272, 275]),
        (vec![104], true, vec![]),
        (vec![], false, vec![]),
    ] {
        let mut i = death_inputs(0, 5);
        i.tables.monsters[0].as_mut().unwrap().height = 1;
        i.tables.missiles = vec![ClientMissileRow::default(); 430];
        i.tables.states = vec![StateRow::default(); 185];
        for s in [1, 107, 184] {
            i.tables.states[s] = both;
        }
        i.tables.states[104].hide = true;
        let mut w = world_with(k, 1);
        w.units.get_mut(&k).unwrap().flag_2 = Some(true);
        w.units
            .get_mut(&k)
            .unwrap()
            .states
            .extend(states.iter().copied());
        req(&mut w, &i, k, 0x08, [0, 0, 0, 0, 0, 0, 0x2D]).unwrap();
        let u = &w.units[&k];
        assert_eq!(u.flag_ex & 0x40000 != 0, hidden, "states {states:?}");
        assert_eq!(u.flag_20, hidden, "states {states:?}");
        assert_eq!(u.flag_2, Some(false), "states {states:?}");
        let shattered = !missiles.is_empty();
        assert_eq!(
            u.hit_class,
            if shattered { 0xAD } else { 0x2D },
            "states {states:?}"
        );
        let made: Vec<u32> = w
            .objclient
            .set_c
            .values()
            .filter(|u| u.key.unit_type == MISSILE)
            .map(|u| u.class)
            .collect();
        assert_eq!(made, missiles, "states {states:?}");
    }
}

/// The kind-2 overlays with (U, id, 0) before D0, by
/// old BaseId (212: 188 then 189; 497 by class: 498 → 226).
#[test]
fn the_death_switch_creates_the_base_id_overlays() {
    let k = UnitKey::new(MONSTER, 5);
    for (class, base_id, ids) in [
        (0, 127, vec![79]),
        (0, 212, vec![188, 189]),
        (498, 497, vec![226]),
        (0, 497, vec![]),
        (0, 110, vec![146]),
        (0, 5, vec![]),
    ] {
        let i = death_inputs(class, base_id);
        let mut w = world_with(k, 1);
        w.units.get_mut(&k).unwrap().class = class as u32;
        req(&mut w, &i, k, 0x08, R0).unwrap();
        let got: Vec<(u16, u8)> = w.hit_overlays.iter().map(|h| (h.id, h.kind)).collect();
        let want: Vec<(u16, u8)> = ids.iter().map(|&id| (id, 2)).collect();
        assert_eq!(got, want, "class {class} base {base_id}");
        assert_eq!(w.units[&k].mode, 0, "class {class} base {base_id}");
    }
}

/// Code 9 sets flags-ex 0x40000 for BaseId 344, 559, 570 and
/// clears the selectable bit for every monster.
#[test]
fn code_9_hides_344_559_570() {
    let k = UnitKey::new(MONSTER, 5);
    for (base_id, flag_ex) in [(344, 0x40000), (559, 0x40000), (570, 0x40000), (5, 0)] {
        let i = death_inputs(0, base_id);
        let mut w = world_with(k, 1);
        w.units.get_mut(&k).unwrap().flag_2 = Some(true);
        req(&mut w, &i, k, 0x09, R0).unwrap();
        let u = &w.units[&k];
        assert_eq!(u.mode, 12, "base {base_id}");
        assert_eq!(u.flag_ex & 0x40000, flag_ex, "base {base_id}");
        assert_eq!(u.flag_2, Some(false), "base {base_id}");
    }
}

/// Player codes 0x08 and 0x09 end in the player
/// death, which sets stat 6 (life) to 0 (measured: life 0 with the
/// death mode, before the server's 0x95).
#[test]
fn player_death_codes_clear_life() {
    let i = ModelInputs::default();
    for code in [0x08, 0x09] {
        let mut w = world_with(P, 1);
        w.units.get_mut(&P).unwrap().stats.insert(6, 5 << 8);
        req(&mut w, &i, P, code, R0).unwrap();
        assert_eq!(w.units[&P].stat(6), 0, "code {code:#x}");
    }
}

/// Item types of the code 0x12 test: `shld` is type 51; `claw` is type
/// 45 `weap` with component 5.
struct BlockItems;
impl super::item_lists::StreamProps for BlockItems {
    fn props(&self, _: &[u8]) -> (Vec<super::item_lists::ItemProp>, bool) {
        (Vec::new(), false)
    }
    fn item_is_type(&self, code: [u8; 4], t: i16) -> bool {
        match &code {
            b"shld" => t == 51,
            b"claw" => t == 45,
            _ => false,
        }
    }
    fn type_class(&self, _: [u8; 4]) -> u32 {
        0
    }
    fn component(&self, code: [u8; 4]) -> u8 {
        if &code == b"claw" {
            5
        } else {
            0
        }
    }
}

/// An item stream head with flags `f` at body
/// location `loc` (mode 1).
fn body_stream(f: u32, loc: u8, code: &[u8; 4]) -> Vec<u8> {
    let bits: [(u32, u32); 8] = [
        (f, 32),
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

/// The local player `P` of class `class` in mode `mode`, wearing
/// `(flags, location, code)` items.
fn wearing(class: u8, mode: u32, worn: &[(u32, u8, &[u8; 4])]) -> ClientWorld {
    use super::world::{ItemData, ItemRecord, KindData};
    let mut w = world_with(P, mode);
    w.units.get_mut(&P).unwrap().class = class.into();
    w.local_player = Some(P);
    w.item_tables = super::item_lists::ItemTablesRef(Some(std::sync::Arc::new(BlockItems)));
    for (n, &(f, loc, code)) in worn.iter().enumerate() {
        let ik = UnitKey::new(ITEM, 9 + n as u32);
        let mut i = ClientUnit::new(ik);
        i.kind = KindData::Item(ItemData {
            last: Some(ItemRecord {
                id: 0x9C,
                action: 0x04,
                category: 0,
                owner: None,
                seq: 0,
                stream: body_stream(f, loc, code),
            }),
            ..Default::default()
        });
        w.units.insert(ik, i);
    }
    w
}

// Code 0x12 sets mode 9 when finds a valid shield or the
// COF weapon class is `ht2` (two claws on an Assassin); otherwise the
// mode is kept.
#[test]
fn code_0x12_blocks_with_a_shield_or_two_claws() {
    let i = ModelInputs::default();
    let mode_after = |mut w: ClientWorld| {
        req(&mut w, &i, P, 0x12, R0).unwrap();
        w.units[&P].mode
    };
    // A valid shield in the left hand.
    assert_eq!(mode_after(wearing(4, 7, &[(0x10, 5, b"shld")])), 9);
    // Unidentified still counts (has no F 0x10 test).
    assert_eq!(mode_after(wearing(4, 7, &[(0, 5, b"shld")])), 9);
    // A broken shield (F 0x100) does not.
    assert_eq!(mode_after(wearing(4, 7, &[(0x110, 5, b"shld")])), 7);
    let claws: &[(u32, u8, &[u8; 4])] = &[(0x10, 4, b"claw"), (0x10, 5, b"claw")];
    // An Assassin with a claw in each hand: `ht2`.
    assert_eq!(mode_after(wearing(6, 7, claws)), 9);
    // Another class with the same claws, or one claw: no.
    assert_eq!(mode_after(wearing(4, 7, claws)), 7);
    assert_eq!(mode_after(wearing(6, 7, &claws[..1])), 7);
    // Mode 0x11 draws `hth` whatever the hands hold.
    assert_eq!(mode_after(wearing(6, 0x11, claws)), 0x11);
}

/// code 7: flag 0x20 := not `monstats2` `shadow`, so a revived
/// monster whose held death set it casts a shadow again; the other codes
/// (here the point group's S1, Resurrect's request) leave it.
#[test]
fn monster_code_7_rewrites_the_no_shadow_flag() {
    use super::world::MonsterSetup;
    let mut i = monster_inputs();
    if let Some(c) = i.tables.monsters[0].as_mut() {
        c.setup = Some(MonsterSetup {
            shadow: true,
            ..MonsterSetup::default()
        });
    }
    let k = UnitKey::new(MONSTER, 5);
    for (code, want) in [(0x07, false), (0x0E, true)] {
        let mut w = world_with(k, 12);
        w.units.get_mut(&k).unwrap().flag_20 = true;
        req(&mut w, &i, k, code, [60, 60, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(w.units[&k].flag_20, want, "code {code:#x}");
    }
}

/// A player's 0x15 / 0x16 for a skill its list lacks (the client skill
/// start, step (a)): the base game adds the skill,
/// looks the (skill, owner) entry up again and uses it; the local
/// player's own entry gets flag 8 and the request's level. An owner the
/// add does not create (an item's charges) still finds none: the start
/// returns 0 and the mode stays.
#[test]
fn a_player_skill_start_adds_a_missing_skill() {
    use super::skills::{used_entry, SkillList, NATIVE};
    use super::world::SkillRow;
    let mut i = ModelInputs::default();
    i.tables.skills = vec![
        SkillRow::default(),
        SkillRow {
            anim: 10,
            maxlvl: 20,
            ..SkillRow::default()
        },
    ];
    let mut w = world_with(P, 1);
    w.local_player = Some(P);
    w.units.get_mut(&P).unwrap().skills = Some(SkillList::default());
    req(&mut w, &i, P, 0x15, [1, -1, 50, 50, 5, 0, 0]).unwrap();
    let e = used_entry(&w, P).copied().expect("added and used");
    assert_eq!((e.skill, e.owner, e.base), (1, NATIVE, 5));
    assert_ne!(e.flags & 8, 0, "the local player's own entry");
    assert_eq!(w.units[&P].mode, 10);
    // Owner 7 (no such entry after the add): the start returns 0.
    let mut w = world_with(P, 1);
    w.units.get_mut(&P).unwrap().skills = Some(SkillList::default());
    req(&mut w, &i, P, 0x15, [1, 7, 50, 50, 5, 0, 0]).unwrap();
    assert!(used_entry(&w, P).is_none());
    assert_eq!(w.units[&P].mode, 1);
}

/// Code 9 leaves base id 284 (the Maggot Queen)
/// in mode 0xE with its QueenDeath (216) entry as the used skill; another
/// monster, or 284 in another mode, takes mode 0xC.
#[test]
fn code_9_keeps_the_maggot_queen_in_her_death_sequence() {
    use super::skills::{SkillEntry, SkillList, NATIVE};
    let k = UnitKey::new(MONSTER, 5);
    for (base_id, mode, want_mode, want_skill) in [
        (284, 0xE, 0xE, Some(216)),
        (284, 1, 0xC, None),
        (5, 0xE, 0xC, None),
    ] {
        let i = death_inputs(0, base_id);
        let mut w = world_with(k, mode);
        w.units.get_mut(&k).unwrap().skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: 216,
                owner: NATIVE,
                ..SkillEntry::default()
            }],
            ..SkillList::default()
        });
        req(&mut w, &i, k, 0x09, [0; 7]).unwrap();
        let u = &w.units[&k];
        assert_eq!(u.mode, want_mode, "base {base_id} mode {mode}");
        assert_eq!(u.used_skill, want_skill, "base {base_id} mode {mode}");
        if want_skill.is_some() {
            assert_eq!(u.skills.as_ref().unwrap().current, Some(0));
        }
    }
}

/// A 0x15 / 0x16 request into mode 0xE reloads
/// the sequence also when the monster is already in 0xE (the preload
/// leaves +0x10 at a frame's own mode; MagottUp after MagottDown
/// restarts at frame 0).
#[test]
fn a_sequence_request_reloads_an_unchanged_sequence_mode() {
    use super::skills::{SkillEntry, SkillList, NATIVE};
    let k = UnitKey::new(MONSTER, 8);
    let mut i = death_inputs(0, 68);
    i.tables.skills = vec![super::world::SkillRow::default(); 162];
    let mut w = world_with(k, 0xE);
    {
        let u = w.units.get_mut(&k).unwrap();
        u.frame = 0x500;
        u.skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: 161,
                mode: 0xE,
                owner: NATIVE,
                ..SkillEntry::default()
            }],
            ..SkillList::default()
        });
    }
    let before = w.units[&k].anim_restarts;
    // 0x15 (a point): a 0x16 with no target unit takes the neutral fallback.
    req(&mut w, &i, k, 0x15, [161, -1, 52, 52, 1, 0, 0]).unwrap();
    let u = &w.units[&k];
    assert_eq!((u.mode, u.frame), (0xE, 0));
    assert_eq!(u.anim_restarts, before.wrapping_add(1));
}

/// The dual-wield toggle before a skill start:
/// a Barbarian or an Assassin with a
/// weapon in each hand starting Attack swings Left Hand Swing (skill 5)
/// first, then Attack and Left Hand Swing alternate; the Barbarian's swing
/// is the row's `anim` (S3 15), the Assassin's the entry's S4 16. One
/// weapon, or another class: Attack (measured on the base game).
#[test]
fn a_dual_wield_attack_start_alternates_with_left_hand_swing() {
    use super::skills::{SkillEntry, SkillList, NATIVE};
    use super::world::SkillRow;
    let mut i = ModelInputs::default();
    i.tables.skills = (0..6)
        .map(|s| SkillRow {
            anim: match s {
                0 => 7,
                5 => 15,
                _ => 10,
            },
            ..SkillRow::default()
        })
        .collect();
    let claws: &[(u32, u8, &[u8; 4])] = &[(0x10, 4, b"claw"), (0x10, 5, b"claw")];
    let starts = |class: u8, worn: &[(u32, u8, &[u8; 4])]| {
        let mut w = wearing(class, 1, worn);
        w.units.get_mut(&P).unwrap().skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: 0,
                mode: 7,
                base: 1,
                owner: NATIVE,
                ..SkillEntry::default()
            }],
            ..SkillList::default()
        });
        (0..3)
            .map(|_| {
                req(&mut w, &i, P, 0x15, [0, -1, 60, 50, 0, 0, 0]).unwrap();
                let u = &w.units[&P];
                let l = u.skills.as_ref().unwrap();
                (u.mode, l.current.map(|c| l.entries[c].skill))
            })
            .collect::<Vec<_>>()
    };
    let swing = |m| [(m, Some(5)), (7, Some(0)), (m, Some(5))].to_vec();
    assert_eq!(starts(4, claws), swing(15));
    assert_eq!(starts(6, claws), swing(16));
    assert_eq!(starts(6, &claws[..1]), vec![(7, Some(0)); 3]);
    assert_eq!(starts(1, claws), vec![(7, Some(0)); 3]);
}
