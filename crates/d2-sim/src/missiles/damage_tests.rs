//! Test vectors of the missile damage setup on the skills fake.

use super::*;
use crate::rng::Seed;
use crate::skills::fake::{missile_rec, skill_rec, skill_tables, FUnit, Fake};
use crate::skills::{phys_max, phys_min};

impl SetupWorld for Fake {
    fn setup_weapon(&self, owner: usize) -> Option<usize> {
        self.units[owner].weapon
    }
    fn has_inventory(&self, _: usize) -> bool {
        false
    }
    fn two_handed(&self, _: usize) -> bool {
        false
    }
    fn set_layer_stat(&mut self, u: usize, stat: u16, layer: u16, value: i32) {
        self.log.push(format!("set {u} {stat} {layer} {value}"));
        self.units[u].stats.insert((stat, layer), value);
    }
    fn dual_wield_toggle(&mut self, owner: usize) {
        self.log.push(format!("toggle {owner}"));
    }
}

const SKILL: i32 = 1;
const CLASS: i32 = 1;

/// Tables with skill 1 (`SrcDam` 128, `EType` `etype`, `ELen` `elen`)
/// and missile class 1 (`row`).
fn tables(etype: u8, elen: u32, row: d2_data::tables::Missiles) -> SkillTables {
    let mut s = skill_rec();
    s.srcdam = 128;
    s.etype = etype;
    s.elen = elen;
    let mut t = skill_tables(vec![skill_rec(), s]);
    t.missiles.push(row);
    t
}

fn skill_row() -> d2_data::tables::Missiles {
    let mut m = missile_rec();
    m.skill = SKILL as u16;
    m
}

fn plain_row(src: u8, miss: u8) -> d2_data::tables::Missiles {
    let mut m = missile_rec();
    m.skill = 0xFFFF;
    m.srcdamage = src;
    m.srcmissdmg = miss;
    m
}

/// (fake, owner, missile).
fn world() -> (Fake, usize, usize) {
    let mut w = Fake::default();
    let owner = w.add(FUnit::new(UnitType::Player, 0).with(21, 2).with(22, 5));
    let missile = w.add(FUnit::new(UnitType::Missile, CLASS));
    (w, owner, missile)
}

fn stepped(n: usize) -> Seed {
    let mut s = FUnit::new(UnitType::Player, 0).seed;
    for _ in 0..n {
        s.step();
    }
    s
}

#[test]
fn skill_missile_adds_the_owner_damage_with_one_draw() {
    let t = tables(0, 0, skill_row());
    let (mut w, owner, missile) = world();
    let pmin = phys_min(&mut w, &t, Some(owner), SKILL, 1, false);
    let pmax = phys_max(&mut w, &t, Some(owner), SKILL, 1, false);
    let r = record(&mut w, &t, owner, None, missile, 0, 1);
    assert_eq!(r.phys_min, pmin + 512);
    assert_eq!(r.phys_max, pmax + 1280);
    assert_eq!(r.damage_pct, 0);
    assert_eq!(r.flags & rec_flag::OWNER, rec_flag::OWNER);
    assert_eq!(w.units[owner].seed, stepped(1), "exactly one owner draw");
    assert_eq!(w.units[missile].seed, stepped(0));
}

#[test]
fn stats_land_on_the_missile() {
    let t = tables(0, 0, skill_row());
    let (mut w, owner, missile) = world();
    let bits = setup(&mut w, &t, owner, None, missile, 0, 3);
    assert_eq!(bits, 1, "owner-sourced → data flag 1");
    assert_eq!(w.get(missile, 12), 3);
    assert_eq!(w.get(missile, 21), 512);
    assert_eq!(w.get(missile, 22), 1280);
    assert_eq!(w.get(missile, 25), 0);
}

#[test]
fn origin_sourced_halves_the_origin_damage() {
    let t = tables(0, 0, plain_row(0, 64));
    let (mut w, owner, missile) = world();
    let origin = w.add(
        FUnit::new(UnitType::Monster, 0)
            .with(21, 10)
            .with(22, 20)
            .with(48, 8)
            .with(49, 16)
            .with(57, 30),
    );
    let r = record(&mut w, &t, owner, Some(origin), missile, 0, 1);
    assert_eq!((r.phys_min, r.phys_max), (5, 10));
    assert_eq!((r.fire_min, r.fire_max), (4, 8), "scale(64), no shift");
    assert_eq!(r.poison_min, 15);
    assert_eq!(r.flags & rec_flag::OWNER, 0);
    assert_eq!(w.units[origin].seed, stepped(1), "one origin draw");
    assert_eq!(w.units[owner].seed, stepped(0), "no owner draw");
}

#[test]
fn fire_missile_keeps_only_the_owner_firelength() {
    let t = tables(1, 25, skill_row());
    let (mut w, owner, missile) = world();
    let r = record(&mut w, &t, owner, None, missile, 0, 1);
    assert_eq!(r.fire_len, 25, "the record holds ELen");
    write_stats(&mut w, Some(owner), missile, &r, 1);
    assert_eq!(w.get(missile, 315), 0, "burn length (owner 315 = 0) wins");
    let i = |s: &str| w.log.iter().position(|l| l == s).unwrap();
    assert!(i(&format!("set {missile} 315 0 25")) < i(&format!("set {missile} 315 0 0")));

    let (mut w, owner, missile) = world();
    w.set(owner, 315, 7);
    setup(&mut w, &t, owner, None, missile, 0, 1);
    assert_eq!(w.get(missile, 315), 7);
}

#[test]
fn bonus_draws_once_per_nonzero_source() {
    let t = tables(0, 0, skill_row());
    let (mut w, owner, _) = world();
    // Stat 337 = 100: the first draw is under it.
    w.set(owner, 337, 100);
    assert!(bonus(&mut w, &t, owner, None));
    assert_eq!(w.units[owner].seed, stepped(1));
    // 337 = 0, deadly strike 0, no weapon: one draw, false.
    let (mut w, owner, _) = world();
    assert!(!bonus(&mut w, &t, owner, None));
    assert_eq!(w.units[owner].seed, stepped(1));
    // Deadly strike 1: a second draw.
    let (mut w, owner, _) = world();
    w.set(owner, 141, 1);
    bonus(&mut w, &t, owner, None);
    assert_eq!(w.units[owner].seed, stepped(2));
}

#[test]
fn srcdamage_255_makes_a_skill_missile_unsourced() {
    let mut row = skill_row();
    row.srcdamage = 255;
    let t = tables(0, 0, row);
    let (mut w, owner, missile) = world();
    w.set(missile, 25, -500);
    let r = record(&mut w, &t, owner, None, missile, 0, 1);
    assert_eq!(r.flags & rec_flag::OWNER, 0);
    assert_eq!(r.damage_pct, -90);
    assert_eq!(w.units[owner].seed, stepped(0), "no draw");
}
