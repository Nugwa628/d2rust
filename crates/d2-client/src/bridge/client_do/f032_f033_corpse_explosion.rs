//! cltdofunc 32 (Corpse Explosion) and 33 (Poison Explosion): two
//! functions of the client skill do table (`super::client_do`) that share
//! the corpse read and the no-target sound; the shared helpers live in
//! `super`.

use super::*;

/// A class inside the missiles table and above 0 (the `0 < c < count`
/// tests of functions 32 and 33).
pub(super) fn live_missile(inputs: &ModelInputs, c: i16) -> bool {
    c > 0 && usize::from(c as u16) < inputs.tables.missiles.len()
}

/// `cltdofunc` 32 (Corpse Explosion): (x, y, cls)
/// := U's used entry E +0x18 / +0x1C / +0x20 (`cltstfunc` 21's corpse).
/// A valid row, a := `cltmissilea` valid, x and y ≠ 0 and a `monstats2`
/// record for cls, else 0. Flag 0x40 on U. The record: flags 0x2021,
/// owner and origin U, class a, start (x, y), target (x + roll(3) − 1,
/// y + roll(3) − 1) (two rolls of U's seed). By the corpse's size
/// (`monstats2` +0x05: `small` mask 4, `large` mask 8): small → the
/// spray (s 0, n 1) of a; large → (3, 3) of a, then (2, 4) of a's
/// `SubMissile1` when it is live; else (2, 2) of a, then (1, 3) of a's
/// `SubMissile2` when live. The start is restored to (x, y); the target
/// and the direction callback stay those of the last spray. A live
/// `cltmissileb` → M := its create; a live `cltmissilec` → its create at
/// level 2. No path target, M made and `tgtsound` > 0 → a request of it
/// on M. 1.
pub(super) fn corpse_explosion(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(e) = current_entry(w, key).copied() else {
        return Ok(0);
    };
    let (x, y, cls) = (e.param, e.param2, e.counter);
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).cloned() else {
        return Ok(0);
    };
    let [a, b, c] = row.cltmissile_abc;
    let class = usize::try_from(cls)
        .ok()
        .and_then(|c| inputs.tables.monsters.get(c).copied().flatten());
    let (true, true, true, Some(class)) =
        (valid_missile(inputs, i32::from(a)), x != 0, y != 0, class)
    else {
        return Ok(0);
    };
    set_flag_40(w, key);
    let mut rec = CreateRecord {
        flags: flag::POSITION | flag::TARGET_ABSOLUTE | flag::RANDOM_DIRECTION,
        owner: Some(key),
        origin: Some(key),
        class: a as u32,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    rec.tx = unit_roll(w, key, 3) as i32 - 1 + x;
    rec.ty = unit_roll(w, key, 3) as i32 - 1 + y;
    (rec.x, rec.y) = (x, y);
    let subs = inputs.tables.missiles[a as usize];
    let dir = if class.small {
        spray(w, inputs, key, &mut rec, 0, 1)?
    } else {
        let (s, n, sub, s2, n2) = if class.large {
            (3, 3, subs.sub1_server, 2, 4)
        } else {
            (2, 2, subs.sub2_server, 1, 3)
        };
        let d = spray(w, inputs, key, &mut rec, s, n)?;
        if live_missile(inputs, sub) {
            rec.class = sub as u32;
            spray(w, inputs, key, &mut rec, s2, n2)?
        } else {
            d
        }
    };
    (rec.x, rec.y) = (x, y);
    let mut made = None;
    for (class, lvl) in [(b, None), (c, Some(2))] {
        if !live_missile(inputs, class) {
            continue;
        }
        rec.class = class as u32;
        if let Some(l) = lvl {
            rec.level = l;
        }
        let k = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
        if let Some(m) = k.and_then(|k| w.objclient.missiles.get_mut(&k)) {
            m.direction = dir;
        }
        if lvl.is_none() {
            made = k;
        }
    }
    target_sound(w, key, &row, made);
    Ok(1)
}

/// The no-target sound of functions 32 and 33: U without a path target,
/// M made and `tgtsound` > 0 → the sound `tgtsound` plays at M.
pub(super) fn target_sound(
    w: &mut ClientWorld,
    key: UnitKey,
    row: &SkillRow,
    made: Option<UnitKey>,
) {
    if let (None, Some(m), true) = (path_target_unit(w, key), made, row.tgtsound > 0) {
        w.objclient.missile_sounds.push(MissileSound::Request {
            id: i32::from(row.tgtsound),
            missile: m,
        });
    }
}

/// `cltdofunc` 33 (Poison Explosion): (x, y, cls)
/// from U's used entry as function 32. A valid row, a := `cltmissilea`
/// valid, cls inside the `monstats` table and x, y ≠ 0, else 0. Flag 0x40
/// on U; M := the create of flags 0x2001, owner U, class a, start (x,
/// y). A live `cltmissilec` → its create at level 2. With M: a live
/// `cltmissileb` → the ring with (M, b, 0, 2, 0) (eight creates),
/// then the no-target sound of function 32 on M. 1.
pub(super) fn poison_explosion(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    level: i32,
) -> Result<i32, HandlerError> {
    let Some(e) = current_entry(w, key).copied() else {
        return Ok(0);
    };
    let (x, y, cls) = (e.param, e.param2, e.counter);
    let Some(row) = inputs.tables.skills.get(usize::from(skill)).cloned() else {
        return Ok(0);
    };
    let [a, b, c] = row.cltmissile_abc;
    let monstats = usize::try_from(cls).is_ok_and(|c| c < inputs.tables.monsters.len());
    if !valid_missile(inputs, i32::from(a)) || !monstats || x == 0 || y == 0 {
        return Ok(0);
    }
    set_flag_40(w, key);
    let mut rec = CreateRecord {
        flags: flag::POSITION | flag::RANDOM_DIRECTION,
        owner: Some(key),
        class: a as u32,
        x,
        y,
        skill: i32::from(skill),
        level,
        owner_dir64: owner_dir64(w, key),
        ..CreateRecord::default()
    };
    let made = create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    if live_missile(inputs, c) {
        (rec.class, rec.level) = (c as u32, 2);
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    }
    if let Some(m) = made {
        if live_missile(inputs, b) {
            crate::bridge::client_missiles::ring(w, &search_env(inputs), m, i32::from(b), 0, 2, 0)?;
        }
        target_sound(w, key, &row, made);
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    fn explosion_inputs(f: i16) -> ModelInputs {
        use crate::bridge::world::MonsterClass;
        let mut i = inputs(f);
        let row = &mut i.tables.skills[usize::from(FROST_NOVA)];
        row.cltmissile_abc = [1, 3, 4];
        row.tgtsound = 7;
        i.tables.missiles = vec![ClientMissileRow::default(); 5];
        i.tables.missiles[1].sub1_server = 2;
        i.tables.missiles[1].sub2_server = 2;
        i.tables.missiles[3].param = [5, 0];
        i.tables.monsters = vec![None, None, Some(MonsterClass::default())];
        i
    }

    #[test]
    fn corpse_explosion_sprays_by_the_corpse_size_then_b_and_c() {
        use crate::bridge::client_missiles::MissileSound;
        use crate::bridge::world::MonsterClass;
        // Medium: (2, 2) of a, (1, 3) of a's SubMissile2, then b and c
        // (level 2) at the corpse, with the last spray's direction. Each
        // spray start is the record's start ± s (roll 2s − s per axis); the
        // second spray goes on from the first one's last start.
        let (mut w, key) = world();
        let i = explosion_inputs(32);
        corpse_entry(&mut w, key, 5010, 5020, 2);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 6).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let got = made(&w);
        let shape: Vec<_> = got.iter().map(|m| (m.0, m.2)).collect();
        assert_eq!(
            shape,
            [(1, 6), (1, 6), (2, 6), (2, 6), (2, 6), (3, 6), (4, 2)]
        );
        let near = |m: &(u32, (u16, u16), i32, u8), o: (u16, u16), s: i32| {
            let d = |a: u16, b: u16| i32::from(a) - i32::from(b);
            let r = -s..s;
            r.contains(&d(m.1 .0, o.0)) && r.contains(&d(m.1 .1, o.1))
        };
        assert!(got[..2].iter().all(|m| near(m, (5010, 5020), 2)), "{got:?}");
        assert!(got[2..5].iter().all(|m| near(m, got[1].1, 1)), "{got:?}");
        // Each spray turns a quarter per create.
        assert_eq!(got[1].3, (got[0].3 + 16) & 63);
        assert_eq!(got[3].3, (got[2].3 + 16) & 63);
        assert_eq!(got[4].3, (got[3].3 + 16) & 63);
        for m in &got[5..] {
            assert_eq!((m.1, m.3), ((5010, 5020), got[4].3));
        }
        // No path target: `tgtsound` on b's missile.
        let b = *w.objclient.set_c.keys().nth(5).expect("b");
        assert!(w
            .objclient
            .missile_sounds
            .contains(&MissileSound::Request { id: 7, missile: b }));
        // Small: one create of a at the corpse (s = 0), then b and c.
        let (mut w, key) = world();
        let mut i = explosion_inputs(32);
        i.tables.monsters[2] = Some(MonsterClass {
            small: true,
            ..MonsterClass::default()
        });
        corpse_entry(&mut w, key, 5010, 5020, 2);
        client_do(&mut w, &i, key, FROST_NOVA, 6).unwrap();
        let got: Vec<_> = made(&w).iter().map(|m| (m.0, m.1)).collect();
        assert_eq!(
            got,
            [(1, (5010, 5020)), (3, (5010, 5020)), (4, (5010, 5020))]
        );
        // Large: (3, 3) of a, (2, 4) of SubMissile1.
        let (mut w, key) = world();
        i.tables.monsters[2] = Some(MonsterClass {
            large: true,
            ..MonsterClass::default()
        });
        corpse_entry(&mut w, key, 5010, 5020, 2);
        client_do(&mut w, &i, key, FROST_NOVA, 6).unwrap();
        let classes: Vec<_> = made(&w).iter().map(|m| m.0).collect();
        assert_eq!(classes, [1, 1, 1, 2, 2, 2, 2, 3, 4]);
        // A point coordinate 0 or no `monstats2` record: 0, no flag.
        for (x, cls) in [(0, 2), (5010, 1), (5010, 9)] {
            let (mut w, key) = world();
            corpse_entry(&mut w, key, x, 5020, cls);
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 6).unwrap(), 0);
            assert!(!w.units[&key].flag_40 && made(&w).is_empty());
        }
    }

    #[test]
    fn poison_explosion_makes_a_then_c_then_the_ring_of_b() {
        let (mut w, key) = world();
        let i = explosion_inputs(33);
        corpse_entry(&mut w, key, 5010, 5020, 1);
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 4).unwrap(), 1);
        assert!(w.units[&key].flag_40);
        let classes: Vec<_> = made(&w).iter().map(|m| (m.0, m.2)).collect();
        // a, c at level 2, then the eight ring creates of b (i = 0, 2, …,
        // 14) at M's level.
        let mut want = vec![(1, 4), (4, 2)];
        want.extend([(3, 4); 8]);
        assert_eq!(classes, want);
        // The ring's velocity: b's `Param1` << 7.
        // The ring's velocity: b's `Param1` << 7, through the create's
        // slow scaling (× 75 / 100, as the nova ring).
        let v: Vec<_> = w
            .objclient
            .set_c
            .keys()
            .map(|k| w.objclient.missiles[k].velocity)
            .collect();
        assert!(v[2..].iter().all(|&v| v == (5 << 7) * 75 / 100), "{v:?}");
        // A class outside `monstats` or a point coordinate 0: 0.
        for (y, cls) in [(5020, 3), (0, 1)] {
            let (mut w, key) = world();
            corpse_entry(&mut w, key, 5010, y, cls);
            assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 4).unwrap(), 0);
            assert!(made(&w).is_empty());
        }
    }
}
