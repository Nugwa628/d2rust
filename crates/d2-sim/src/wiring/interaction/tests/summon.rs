//! Tests for a summon skill on the wired host: the druid summon body (`srvdo`
//! 119) makes a real monster at the aimed point, adds it to the owner's
//! pet list ([`crate::wiring::action::ActionHooks::pet_lists`]) and
//! broadcasts 0x7A; a summon past the pet type's maximum removes the
//! oldest pet; a dead pet leaves its list ([`View::pet_sweep`]).

use std::sync::Arc;

use crate::skills::use_::bodies::{BodyStat, BodyTables};
use crate::skills::use_::do_skill;
use crate::skills::SkillEntry;
use crate::units::modes::monster_mode;
use crate::units::{UnitId, UnitType};

use super::skill_bodies::body_rec;
use super::skill_use::Fx;
use super::{stat_data, N_STATES};

/// Formulas (`push i16 v; end`): 2, 1, 5.
const F_2: u32 = 0;
const F_1: u32 = 4;
const F_5: u32 = 8;
/// `lvl` (skill special 16): the skill's level.
const F_LVL: u32 = 12;

/// Pet type 2, whose maximum is `petmax` (class `summon` 0).
fn summoner(petmax: u32) -> (Fx, UnitId) {
    let mut r = body_rec();
    r.srvdofunc = 119;
    r.summon = 0;
    r.summode = 1;
    r.pettype = 2;
    r.petmax = petmax;
    r.calc2 = F_5;
    let mut t = crate::skills::fake::skill_tables(vec![r]);
    let mut code = Vec::new();
    for v in [2i16, 1, 5] {
        code.push(0x08);
        code.extend(v.to_le_bytes());
        code.push(0x00);
    }
    code.extend([0x04, 16, 0x00]);
    t.skills_code = code;
    let mut fx = Fx::with(stat_data(), t);
    fx.sim.sys.hooks.bodies = Some(Arc::new(BodyTables {
        stats: vec![BodyStat::default(); 359],
        state_group: vec![0; N_STATES],
        state_aura: vec![false; N_STATES],
        pettype_count: 3,
        pettype_group: vec![0; 3],
        ..BodyTables::default()
    }));
    let p = fx.spawn(UnitType::Player, 10, 10);
    fx.set(p, &[(12, 10)]);
    (fx, p)
}

fn cast(fx: &mut Fx, p: UnitId, at: (i32, i32)) -> i32 {
    let e = SkillEntry {
        skill: 0,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    let x = &mut fx.sim.hooks().x;
    x.skills.insert(p, vec![e]);
    x.used.insert(p, e);
    x.aim_at = at;
    let t = fx.sim.sys.hooks.tables.skills.clone();
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| do_skill(w, &t, p, 0, 1))
}

fn pets(fx: &Fx, p: UnitId) -> Vec<i32> {
    fx.sim
        .sys
        .hooks
        .pet_lists
        .get(&p)
        .map(|l| l.entries[2].nodes.iter().map(|n| n.guid).collect())
        .unwrap_or_default()
}

/// The 0x7A messages sent so far: (action, pet type, pet GUID).
fn pet_msgs(fx: &mut Fx) -> Vec<(u8, u8, u32)> {
    fx.sim
        .hooks()
        .x
        .msgs
        .iter()
        .filter(|(_, b)| b[0] == 0x7A)
        .map(|(_, b)| (b[1], b[2], u32::from_le_bytes(b[9..13].try_into().unwrap())))
        .collect()
}

#[test]
fn a_summon_makes_a_monster_and_lists_it_with_a_0x7a_add() {
    let (mut fx, p) = summoner(F_2);
    assert_eq!(cast(&mut fx, p, (12, 10)), 1);
    let m = fx.game.lists.units_of_type(UnitType::Monster);
    assert_eq!(m.len(), 1, "the summon allocated one monster");
    let guid = fx.sim.sys.units.get(m[0]).unwrap().guid;
    assert_eq!(pets(&fx, p), vec![guid as i32]);
    assert_eq!(pet_msgs(&mut fx), vec![(1, 2, guid)]);
    fx.assert_clean();
}

#[test]
fn a_summon_past_the_maximum_removes_the_oldest_pet() {
    let (mut fx, p) = summoner(F_2);
    cast(&mut fx, p, (12, 10));
    cast(&mut fx, p, (13, 10));
    let two = pets(&fx, p);
    assert_eq!(two.len(), 2);
    cast(&mut fx, p, (14, 10));
    let now = pets(&fx, p);
    assert_eq!(now.len(), 2, "the maximum is 2");
    assert_eq!(now[0], two[1], "the oldest left, the next is the head");
    assert!(
        pet_msgs(&mut fx)
            .iter()
            .any(|&(a, _, g)| a == 0 && g as i32 == two[0]),
        "0x7A remove for the oldest"
    );
    fx.assert_clean();
}

#[test]
fn a_dead_pet_leaves_the_list() {
    let (mut fx, p) = summoner(F_1);
    cast(&mut fx, p, (12, 10));
    let m = fx.game.lists.units_of_type(UnitType::Monster)[0];
    fx.sim.sys.units.get_mut(m).unwrap().mode = monster_mode::DT;
    let game = &mut fx.game;
    fx.sim.with(game, |g, v| v.pet_sweep(g));
    assert!(pets(&fx, p).is_empty());
    assert!(pet_msgs(&mut fx).iter().any(|&(a, _, _)| a == 0));
}

/// A cast of skill 0 at level `base`.
fn cast_at(fx: &mut Fx, p: UnitId, at: (i32, i32), base: i32) -> i32 {
    let e = SkillEntry {
        skill: 0,
        base,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    let x = &mut fx.sim.hooks().x;
    x.skills.insert(p, vec![e]);
    x.used.insert(p, e);
    x.aim_at = at;
    let t = fx.sim.sys.hooks.tables.skills.clone();
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| do_skill(w, &t, p, 0, base))
}

#[test]
fn a_lower_skill_level_resyncs_the_maximum_and_trims_the_list() {
    use crate::skills::use_::bodies::BodyWorld;
    let (mut fx, p) = summoner(F_LVL);
    for x in 12..15 {
        cast_at(&mut fx, p, (x, 10), 3);
    }
    let three = pets(&fx, p);
    assert_eq!(three.len(), 3, "petmax = level 3");
    assert_eq!(fx.sim.sys.hooks.pet_lists[&p].entries[2].max, 3);
    // The level drops (an item bonus removed): the resync takes the
    // maximum of the skills' petmax (1) and trims with kill 1.
    fx.sim.hooks().x.skills.get_mut(&p).unwrap()[0].base = 1;
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| BodyWorld::skill_resync(w, p));
    let e = &fx.sim.sys.hooks.pet_lists[&p].entries[2];
    assert_eq!((e.max, e.count), (1, 1));
    assert_eq!(pets(&fx, p), vec![three[2]], "the oldest two left");
    let gone: Vec<i32> = pet_msgs(&mut fx)
        .iter()
        .filter(|&&(a, _, _)| a == 0)
        .map(|&(_, _, g)| g as i32)
        .collect();
    assert!(gone.contains(&three[0]) && gone.contains(&three[1]));
    // Raised again: the maximum grows, nothing is added.
    fx.sim.hooks().x.skills.get_mut(&p).unwrap()[0].base = 4;
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| BodyWorld::skill_resync(w, p));
    assert_eq!(fx.sim.sys.hooks.pet_lists[&p].entries[2].max, 4);
    assert_eq!(pets(&fx, p).len(), 1);
    fx.assert_clean();
}

#[test]
fn a_summons_aura_sumskill_becomes_its_right_skill_with_the_aura_running() {
    // Skill 1 summons with `sumskill1` = skill 2 at `eval(sumsk1calc)` 1
    // (skill 0 is skipped); skill 2 is an `aura` (state 30, not
    // `immediate`): the monster gets a skill list with skill 2 on the
    // right and the type-8 aura timer (arg −1), as the monster-aura
    // assign does (its aura state is the host's `set_aura_state`).
    const SUMMON: i32 = 1;
    const AURA: i32 = 2;
    const AURA_STATE: u16 = 30;
    let mut r = body_rec();
    r.srvdofunc = 119;
    r.summon = 0;
    r.summode = 1;
    r.pettype = 2;
    r.petmax = F_2;
    r.calc2 = F_5;
    r.sumskill1 = AURA as u16;
    r.sumsk1calc = F_1;
    let mut a = body_rec();
    a.aura = true;
    a.aurastate = AURA_STATE;
    let mut t = crate::skills::fake::skill_tables(vec![body_rec(), r, a]);
    let mut code = Vec::new();
    for v in [2i16, 1, 5] {
        code.push(0x08);
        code.extend(v.to_le_bytes());
        code.push(0x00);
    }
    t.skills_code = code;
    let mut fx = Fx::with(stat_data(), t);
    fx.sim.sys.hooks.bodies = Some(Arc::new(BodyTables {
        stats: vec![BodyStat::default(); 359],
        state_group: vec![0; N_STATES],
        state_aura: vec![false; N_STATES],
        pettype_count: 3,
        pettype_group: vec![0; 3],
        ..BodyTables::default()
    }));
    let p = fx.spawn(UnitType::Player, 10, 10);
    fx.set(p, &[(12, 10)]);
    let e = SkillEntry {
        skill: SUMMON,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    let x = &mut fx.sim.hooks().x;
    x.skills.insert(p, vec![e]);
    x.used.insert(p, e);
    x.aim_at = (12, 10);
    let t = fx.sim.sys.hooks.tables.skills.clone();
    let game = &mut fx.game;
    assert_eq!(fx.sim.skill_use(game, |w| do_skill(w, &t, p, SUMMON, 1)), 1);
    let m = fx.game.lists.units_of_type(UnitType::Monster)[0];
    let list = fx.sim.sys.hooks.skill_lists.get(&m).expect("a skill list");
    let right = list.right.map(|i| list.view()[i]);
    assert_eq!(right.map(|e| (e.skill, e.owner_guid)), Some((AURA, -1)));
    let periodic = fx
        .game
        .timers
        .unit_timers(m)
        .into_iter()
        .filter_map(|t| fx.game.timers.event(t))
        .any(|e| e.0 == crate::tick::events::event::PERIODIC_SKILLS);
    assert!(periodic, "the type-8 aura timer");
    fx.assert_clean();
}

#[test]
fn a_raised_corpse_leaves_its_room_with_a_delete_record() {
    use crate::skills::use_::bodies::{BodyEffect, BodyWorld};
    let (mut fx, _p) = summoner(F_2);
    let c = fx.spawn(UnitType::Monster, 12, 10);
    let guid = fx.sim.sys.units.get(c).unwrap().guid;
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| {
        w.effect(BodyEffect::RoomDelete(c));
        w.effect(BodyEffect::RemoveUnit(c));
    });
    assert!(fx.sim.sys.units.get(c).is_none(), "the corpse is freed");
    assert!(fx.game.lists.units_of_type(UnitType::Monster).is_empty());
    let recs: Vec<(u8, u32)> = fx
        .sim
        .sys
        .hooks
        .room_deletes
        .values()
        .flatten()
        .copied()
        .collect();
    assert_eq!(
        recs,
        vec![(UnitType::Monster as u8, guid)],
        "one monster room-deletion record"
    );
    fx.assert_clean();
}

#[test]
fn the_iron_golem_takes_its_ground_item_onto_its_body() {
    use crate::skills::use_::bodies::{BodyEffect, BodyWorld};
    let (mut fx, _p) = summoner(F_2);
    let m = fx.spawn(UnitType::Monster, 12, 10);
    let k = fx.spawn(UnitType::Item, 13, 10);
    let guid = fx.sim.sys.units.get(k).unwrap().guid;
    let tile = crate::units::record::flags::TILE;
    assert_ne!(
        fx.sim.sys.units.get(k).unwrap().flags & tile,
        0,
        "a skill target"
    );
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| {
        w.effect(BodyEffect::ItemLeaveRoom(k));
        w.effect(BodyEffect::ItemMode { item: k, mode: 4 });
        w.effect(BodyEffect::Equip { m, item: k, loc: 4 });
    });
    let recs: Vec<(u8, u32)> = fx
        .sim
        .sys
        .hooks
        .room_deletes
        .values()
        .flatten()
        .copied()
        .collect();
    assert_eq!(recs, vec![(UnitType::Item as u8, guid)], "the 0x0A record");
    assert!(
        fx.game.lists.unit(k).and_then(|e| e.room()).is_none(),
        "off its room"
    );
    let r = fx.sim.sys.units.get(k).unwrap();
    assert_eq!(
        (r.mode, r.flags & tile),
        (1, 0),
        "equipped, no longer a target"
    );
    let held = fx
        .sim
        .sys
        .hooks
        .monster_equip
        .get(&m)
        .and_then(|h| h.get(&4));
    assert_eq!(held, Some(&k));
    fx.assert_clean();
}

#[test]
fn a_summon_joins_its_owners_target_node_list() {
    let (mut fx, p) = summoner(F_2);
    fx.sim.sys.units.get_mut(p).unwrap().node_index = 0;
    assert_eq!(cast(&mut fx, p, (12, 10)), 1);
    let a = fx.game.lists.units_of_type(UnitType::Monster)[0];
    cast(&mut fx, p, (13, 10));
    let b = *fx
        .game
        .lists
        .units_of_type(UnitType::Monster)
        .iter()
        .find(|&&m| m != a)
        .unwrap();
    assert_eq!(
        fx.game.target_nodes.slot(0),
        &[b, a],
        "newest right after the head"
    );
    assert_eq!(fx.sim.sys.units.get(a).unwrap().node_index, 0);
    fx.assert_clean();
}

#[test]
fn a_converted_monster_heads_list_9_and_leaves_it_on_remove() {
    use crate::skills::use_::bodies::{BodyEffect, BodyWorld};
    let (mut fx, _p) = summoner(F_2);
    let a = fx.spawn(UnitType::Monster, 12, 10);
    let b = fx.spawn(UnitType::Monster, 13, 10);
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| {
        w.effect(BodyEffect::NodePrepend { u: a, slot: 9 });
        w.effect(BodyEffect::NodePrepend { u: b, slot: 9 });
    });
    assert_eq!(
        fx.game.target_nodes.slot(9),
        &[b, a],
        "prepended: newest first"
    );
    assert_eq!(fx.sim.sys.units.get(b).unwrap().node_index, 9);
    let game = &mut fx.game;
    fx.sim
        .skill_use(game, |w| w.effect(BodyEffect::NodeRemove(b)));
    assert_eq!(fx.game.target_nodes.slot(9), &[a]);
    assert_eq!(
        fx.sim.sys.units.get(b).unwrap().node_index,
        crate::units::record::INITIAL_NODE_INDEX
    );
    fx.assert_clean();
}

#[test]
fn a_freed_monster_frees_the_items_it_holds() {
    use crate::skills::use_::bodies::{BodyEffect, BodyWorld};
    let (mut fx, _p) = summoner(F_2);
    let m = fx.spawn(UnitType::Monster, 12, 10);
    let k = fx.spawn(UnitType::Item, 13, 10);
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| {
        w.effect(BodyEffect::ItemLeaveRoom(k));
        w.effect(BodyEffect::ItemMode { item: k, mode: 4 });
        w.effect(BodyEffect::Equip { m, item: k, loc: 4 });
    });
    assert!(fx.sim.sys.hooks.monster_equip.contains_key(&m));
    let game = &mut fx.game;
    fx.sim
        .skill_use(game, |w| w.effect(BodyEffect::RemoveUnit(m)));
    assert!(fx.sim.sys.units.get(m).is_none(), "the monster is freed");
    assert!(
        fx.sim.sys.units.get(k).is_none() && fx.game.lists.unit(k).is_none(),
        "its item goes with it"
    );
    assert!(!fx.sim.sys.hooks.items.contains(k), "and its item data");
    assert!(
        !fx.sim.sys.hooks.monster_equip.contains_key(&m),
        "no holdings left"
    );
    fx.assert_clean();
}
