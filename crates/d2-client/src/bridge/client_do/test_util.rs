//! Shared fixtures of the client do tests (one test module per function).

pub(super) use super::*;
pub(super) use crate::bridge::client_missiles::ClientMissileRow;
pub(super) use crate::bridge::player_anim::{self, PlayerAnim, PlayerAnims};
pub(super) use crate::bridge::skills::{SkillEntry, SkillList, NATIVE};
pub(super) use crate::bridge::world::{ClientUnit, SkillRow, MISSILE, MONSTER};
pub(super) use std::sync::Arc;

pub(super) const FROST_NOVA: u16 = 44;

pub(super) const CLASS: u32 = 3;

pub(super) fn inputs(cltdofunc: i16) -> ModelInputs {
    let mut i = ModelInputs::default();
    i.tables.skills = vec![SkillRow::default(); 50];
    i.tables.skills[usize::from(FROST_NOVA)] = SkillRow {
        ingame: true,
        cltdofunc,
        cltmissile: -1,
        cltmissile_abc: [CLASS as i16, -1, -1],
        cltcalc1: u32::MAX,
        ..SkillRow::default()
    };
    i.tables.missiles = vec![ClientMissileRow::default(); 4];
    i.tables.missiles[CLASS as usize] = ClientMissileRow {
        vel: 8,
        vel_lev: 4,
        range: 10,
        ..ClientMissileRow::default()
    };
    i
}

pub(super) fn world() -> (ClientWorld, UnitKey) {
    let mut w = ClientWorld::default();
    let key = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(key);
    u.position = Some((5000, 5000));
    u.path_dir = Some(0);
    u.skills = Some(SkillList {
        entries: vec![SkillEntry {
            skill: FROST_NOVA,
            base: 3,
            owner: NATIVE,
            ..SkillEntry::default()
        }],
        current: Some(0),
        ..SkillList::default()
    });
    w.units.insert(key, u);
    (w, key)
}

pub(super) fn missiles(w: &ClientWorld) -> Vec<&crate::bridge::client_missiles::ClientMissile> {
    w.objclient.missiles.values().collect()
}

/// Cast mode 10 with 6 frames at speed 256, event 2 on frame 2.
#[derive(Debug)]
pub(super) struct Cast;

impl PlayerAnims for Cast {
    fn anim(&self, _: &ClientWorld, _: UnitKey, _: u32) -> Option<PlayerAnim> {
        let mut events = [0; d2_formats::animdata::EVENTS];
        events[2] = 2;
        Some(PlayerAnim {
            frames: 6,
            speed: 256,
            weapon: 0,
            events: Some(events),
        })
    }
}

/// A monster that used Frost Nova's row with `cltmissile` = [`CLASS`]
/// and a null `cltdofunc`, standing at (5000, 5000) with a client path
/// aimed at `to`.
pub(super) fn caster(to: (u16, u16)) -> (ClientWorld, ModelInputs, UnitKey) {
    let mut i = inputs(0);
    i.tables.skills[usize::from(FROST_NOVA)].cltmissile = CLASS as i16;
    let mut w = ClientWorld::default();
    let key = UnitKey::new(MONSTER, 2);
    let mut u = ClientUnit::new(key);
    u.position = Some((5000, 5000));
    u.used_skill = Some(i32::from(FROST_NOVA));
    u.used_skill_level = 3;
    w.units.insert(key, u);
    w.objclient.s_paths.insert(
        key,
        d2_sim::path::record::DynamicPath {
            target_x: to.0,
            target_y: to.1,
            direction: 16,
            ..Default::default()
        },
    );
    (w, i, key)
}

pub(super) fn event(w: &mut ClientWorld, key: UnitKey, byte: u8) {
    w.units.get_mut(&key).unwrap().action_frame = byte;
}

/// Skill tables whose `calc1` (offset 0 of the code) is `n`.
pub(super) fn calc1_tables(n: u8) -> Arc<d2_sim::skills::SkillTables> {
    use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
    fn blank<T: Record>() -> T {
        T::decode(&vec![0u8; T::SIZE])
    }
    let mut row: Skills = blank();
    row.calc1 = 0;
    let mut skills = vec![blank::<Skills>(); 50];
    skills[usize::from(FROST_NOVA)] = row;
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

pub(super) fn aimed_at(t: (u16, u16)) -> (ClientWorld, UnitKey) {
    let (mut w, key) = world();
    w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Point(t.0, t.1));
    (w, key)
}

pub(super) fn inputs_b(cltdofunc: i16) -> ModelInputs {
    let mut i = inputs(cltdofunc);
    i.tables.skills[usize::from(FROST_NOVA)].cltmissile_abc = [-1, CLASS as i16, -1];
    i
}

pub(super) fn aimed_at_cow() -> (ClientWorld, UnitKey, UnitKey) {
    let (mut w, key) = world();
    let cow = UnitKey::new(MONSTER, 8);
    let mut c = ClientUnit::new(cow);
    c.position = Some((5004, 5002));
    w.units.insert(cow, c);
    w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Unit(cow));
    (w, key, cow)
}

pub(super) fn aimed_at_unit() -> (ClientWorld, UnitKey, UnitKey) {
    let (mut w, key) = world();
    let t = UnitKey::new(MONSTER, 77);
    let mut u = ClientUnit::new(t);
    u.position = Some((5010, 5004));
    w.units.insert(t, u);
    w.units.get_mut(&key).unwrap().cast_target = Some(CastTarget::Unit(t));
    (w, key, t)
}

/// Skill tables whose `calc1` is `n` and `calc2` is `n2`.
pub(super) fn calc12_tables(n: u8, n2: u8) -> Arc<d2_sim::skills::SkillTables> {
    use d2_data::tables::{Missiles, Record, Skilldesc, Skills};
    fn blank<T: Record>() -> T {
        T::decode(&vec![0u8; T::SIZE])
    }
    let mut row: Skills = blank();
    (row.calc1, row.calc2) = (0, 3);
    let mut skills = vec![blank::<Skills>(); 50];
    skills[usize::from(FROST_NOVA)] = row;
    Arc::new(d2_sim::skills::SkillTables {
        skills,
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: vec![blank::<Missiles>()],
        skills_code: vec![0x07, n, 0x00, 0x07, n2, 0x00],
        miss_code: Vec::new(),
        level_cap: 99,
        stat_count: 359,
    })
}

pub(super) fn seq15(pos: i32) -> crate::bridge::world::MonsterSequence {
    crate::bridge::world::MonsterSequence {
        frames: vec![
            d2_sim::skills::sequences::SeqFrame {
                mode: 18,
                frame: 0,
                event: 1,
            };
            15
        ],
        length: 15 << 8,
        pos,
        step: 0x100,
        drawn_mode: 18,
    }
}

/// The client missiles in create order: (class, sub-tile, level,
/// direction).
pub(super) fn made(w: &ClientWorld) -> Vec<(u32, (u16, u16), i32, u8)> {
    w.objclient
        .set_c
        .iter()
        .map(|(k, u)| {
            let m = &w.objclient.missiles[k];
            (u.class, u.position.unwrap_or((0, 0)), m.level, m.direction)
        })
        .collect()
}

/// U's used entry as the corpse starts leave it: E +0x18 / +0x1C / +0x20.
pub(super) fn corpse_entry(w: &mut ClientWorld, key: UnitKey, x: i32, y: i32, cls: i32) {
    let e = current_entry_mut(w, key).expect("entry");
    (e.param, e.param2, e.counter) = (x, y, cls);
}
