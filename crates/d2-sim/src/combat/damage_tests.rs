//! Tests for damage rolling, totals, resistance rows, application, timed effects, get-hit
//! and durability, covering rules `tests.rs` does not reach. The cases are synthetic and
//! as small as possible.
//!
//! `Spy` wraps the shared `Fake` (`skills::fake`) and observes what the fake does not:
//! the dual-wield switch, the melee range argument, the last attacker, the record handed
//! to the reaction, alignment, dead units, monster modes, and separate unit and item/skill
//! getters.

use super::*;
use crate::rng::Seed;
use crate::skills::fake::*;
use crate::skills::{SkillEntry, SkillTables, SkillUnits};
use crate::units::UnitType;
use std::cell::RefCell;
use std::collections::BTreeMap;

fn st() -> SkillTables {
    skill_tables(vec![skill_rec()])
}

fn ct() -> CombatTables {
    combat_tables(vec![monster_rec()])
}

fn world() -> Fake {
    Fake {
        hostile: true,
        in_range: true,
        expansion: true,
        ..Fake::default()
    }
}

/// A seed whose next step gives `lo′ = x`.
fn seed_giving(x: u32) -> Seed {
    Seed::new(0, x)
}

#[derive(Debug, Clone, Default)]
struct Spy {
    f: Fake,
    /// Added to the unit getter only.
    unit_only: BTreeMap<(usize, u16), i32>,
    /// Added to the item/skill getter only.
    item_only: BTreeMap<(usize, u16), i32>,
    alignment: BTreeMap<usize, i32>,
    dead: Vec<usize>,
    /// (unit, mode) the monster class lacks.
    no_mode: Vec<(usize, i32)>,
    /// Every melee-range query (attacker, defender, range).
    ranges: RefCell<Vec<(usize, usize, i32)>>,
}

impl Spy {
    fn new() -> Self {
        Self {
            f: world(),
            ..Self::default()
        }
    }
    fn log(&self) -> &[String] {
        &self.f.log
    }
}

impl SkillUnits for Spy {
    type Unit = usize;
    type Item = usize;
    fn unit_type(&self, u: usize) -> UnitType {
        self.f.unit_type(u)
    }
    fn class_id(&self, u: usize) -> i32 {
        self.f.class_id(u)
    }
    fn stat(&self, u: usize, stat: u16, layer: u16) -> i32 {
        let extra = if layer == 0 {
            self.unit_only.get(&(u, stat)).copied().unwrap_or(0)
        } else {
            0
        };
        self.f.stat(u, stat, layer) + extra
    }
    fn item_stat(&self, u: usize, stat: u16, layer: u16) -> i32 {
        let extra = if layer == 0 {
            self.item_only.get(&(u, stat)).copied().unwrap_or(0)
        } else {
            0
        };
        self.f.item_stat(u, stat, layer) + extra
    }
    fn base_stat(&self, u: usize, stat: u16, layer: u16) -> i32 {
        self.f.base_stat(u, stat, layer)
    }
    fn formula_stat(&self, u: usize, stat: u16, mode: i32) -> i32 {
        self.f.formula_stat(u, stat, mode)
    }
    fn stat_entries(&self, u: usize, stat: u16, max: usize) -> Vec<(u16, i32)> {
        self.f.stat_entries(u, stat, max)
    }
    fn has_state(&self, u: usize, state: u16) -> bool {
        self.f.has_state(u, state)
    }
    fn state_stat(&self, u: usize, state: u16, stat: u16) -> Option<i32> {
        self.f.state_stat(u, state, stat)
    }
    fn seed(&mut self, u: usize) -> &mut Seed {
        self.f.seed(u)
    }
    fn skill_list(&self, u: usize) -> Vec<SkillEntry> {
        self.f.skill_list(u)
    }
    fn used_skill(&self, u: usize) -> Option<SkillEntry> {
        self.f.used_skill(u)
    }
    fn current_weapon(&self, u: usize) -> Option<usize> {
        self.f.current_weapon(u)
    }
    fn weapon(&self, u: usize) -> Option<usize> {
        self.f.weapon(u)
    }
    fn item_at(&self, u: usize, loc: u8) -> Option<usize> {
        self.f.item_at(u, loc)
    }
    fn item_is(&self, item: usize, itype: i32) -> bool {
        self.f.item_is(item, itype)
    }
    fn itype_is(&self, itype: i32, parent: i32) -> bool {
        self.f.itype_is(itype, parent)
    }
    fn wield_type(&self, item: usize) -> i32 {
        self.f.wield_type(item)
    }
    fn item_damage(&self, item: usize, max: bool) -> i32 {
        self.f.item_damage(item, max)
    }
    fn str_dex_bonus(&self, item: usize) -> (i32, i32) {
        self.f.str_dex_bonus(item)
    }
    fn item_flag_throw(&self, item: usize) -> bool {
        self.f.item_flag_throw(item)
    }
    fn missile_level(&self, u: usize) -> i32 {
        self.f.missile_level(u)
    }
}

impl CombatWorld for Spy {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn frame(&self) -> i32 {
        self.f.frame()
    }
    fn expansion(&self) -> bool {
        self.f.expansion()
    }
    fn difficulty(&self) -> usize {
        self.f.difficulty()
    }
    fn hit_class_counter(&mut self) -> &mut u8 {
        self.f.hit_class_counter()
    }
    fn ident(&self, u: usize) -> (UnitType, u32) {
        self.f.ident(u)
    }
    fn mode(&self, u: usize) -> i32 {
        self.f.mode(u)
    }
    fn moving_mode(&self, u: usize) -> bool {
        self.f.moving_mode(u)
    }
    fn monster_flag(&self, u: usize, mask: u32) -> bool {
        self.f.monster_flag(u, mask)
    }
    fn is_boss(&self, u: usize) -> bool {
        self.f.is_boss(u)
    }
    fn is_hireling(&self, u: usize) -> bool {
        self.f.is_hireling(u)
    }
    fn is_demon(&self, u: usize) -> bool {
        self.f.is_demon(u)
    }
    fn is_undead(&self, u: usize) -> bool {
        self.f.is_undead(u)
    }
    fn is_prime_evil(&self, u: usize) -> bool {
        self.f.is_prime_evil(u)
    }
    fn is_revived(&self, u: usize) -> bool {
        self.f.is_revived(u)
    }
    fn alignment(&self, u: usize) -> i32 {
        self.alignment.get(&u).copied().unwrap_or(0)
    }
    fn hostile(&self, a: usize, d: usize) -> bool {
        self.f.hostile(a, d)
    }
    fn melee_range(&self, u: usize) -> i32 {
        self.f.melee_range(u)
    }
    fn in_melee_range(&self, a: usize, d: usize, range: i32) -> bool {
        self.ranges.borrow_mut().push((a, d, range));
        self.f.in_melee_range(a, d, range)
    }
    fn has_shield(&self, u: usize) -> bool {
        self.f.has_shield(u)
    }
    fn composit_shield(&self, u: usize) -> bool {
        self.f.composit_shield(u)
    }
    fn weapon_class(&self, u: usize) -> i32 {
        self.f.weapon_class(u)
    }
    fn weapon_hit_class(&self, u: usize) -> u32 {
        self.f.weapon_hit_class(u)
    }
    fn montype_matches(&self, layer: u16, montype: i32) -> bool {
        self.f.montype_matches(layer, montype)
    }
    fn room(&self, u: usize) -> RoomKind {
        self.f.room(u)
    }
    fn is_dead(&self, u: usize) -> bool {
        self.dead.contains(&u)
    }
    fn monster_has_mode(&self, u: usize, mode: i32) -> bool {
        !self.no_mode.contains(&(u, mode))
    }
    fn converted_type(&self, u: usize) -> i32 {
        self.f.converted_type(u)
    }
    fn item_has_durability(&self, item: usize) -> bool {
        self.f.item_has_durability(item)
    }
    fn player_count_bonus(&self, players: i32) -> i32 {
        self.f.player_count_bonus(players)
    }
    fn set_stat(&mut self, u: usize, stat: u16, value: i32) {
        CombatWorld::set_stat(&mut self.f, u, stat, value);
    }
    fn unit_event(&mut self, event: u8, unit: usize, other: usize, r: &mut DamageRecord) {
        self.f.unit_event(event, unit, other, r);
    }
    fn set_state(&mut self, u: usize, state: u16, on: bool) {
        self.f.set_state(u, state, on);
    }
    fn curse(
        &mut self,
        t: usize,
        o: usize,
        state: u16,
        stat: u16,
        value: i32,
        frames: i32,
        skill: i32,
        level: i32,
    ) {
        self.f.curse(t, o, state, stat, value, frames, skill, level);
    }
    fn state_list_expiry(&self, u: usize, state: u16) -> Option<i32> {
        self.f.state_list_expiry(u, state)
    }
    fn set_state_list_expiry(&mut self, u: usize, state: u16, e: i32) {
        self.f.set_state_list_expiry(u, state, e);
    }
    fn create_state_list(&mut self, u: usize, state: u16, owner: usize, e: i32) {
        self.f.create_state_list(u, state, owner, e);
    }
    fn set_state_list_stat(&mut self, u: usize, state: u16, stat: u16, v: i32) {
        self.f.set_state_list_stat(u, state, stat, v);
    }
    fn schedule_timer(&mut self, u: usize, ty: u8, frame: i32) {
        self.f.schedule_timer(u, ty, frame);
    }
    fn cancel_timers(&mut self, u: usize, ty: u8) {
        self.f.cancel_timers(u, ty);
    }
    fn overlay(&mut self, u: usize, id: i32) {
        self.f.overlay(u, id);
    }
    fn refresh_anim_rate(&mut self, u: usize) {
        self.f.refresh_anim_rate(u);
    }
    fn set_last_attacker(&mut self, d: usize, a: usize) {
        self.f.log.push(format!("last {d} {a}"));
    }
    fn monster_hit_hook(&mut self, a: usize) {
        self.f.monster_hit_hook(a);
    }
    fn monster_damaged_hook(&mut self, d: usize) {
        self.f.monster_damaged_hook(d);
    }
    fn dual_wield_switch(&mut self, a: usize, offhand: bool, on: bool) {
        self.f.log.push(format!("dual {a} {offhand} {on}"));
    }
    fn combat_list(&mut self, u: usize) -> &mut Vec<CombatEntry> {
        self.f.combat_list(u)
    }
    fn durability_loss(&mut self, owner: usize, item: usize) {
        self.f.durability_loss(owner, item);
    }
    fn thorns(&mut self, a: usize, d: usize, r: &mut DamageRecord) {
        self.f.thorns(a, d, r);
    }
    fn reaction(&mut self, a: usize, d: usize, r: &mut DamageRecord) {
        self.f.log.push(format!(
            "reaction {a} {d} {:#x} {:#x}",
            r.hit_class, r.result
        ));
    }
}

// ================================================================ scale to rolling

#[test]
fn scale_truncates_toward_zero_and_wraps() {
    // `(x × s) / 128` truncates toward zero: −192 / 128 = −1 (a shift
    // would give −2).
    assert_eq!(scale(-3, 64), -1);
    assert_eq!(scale(255, 1), 1);
    assert_eq!(scale(3_840, 64), 1_920);
    // 32-bit product: 0x0200_0000 × 128 wraps to 0.
    assert_eq!(scale(0x0200_0000, 128), 0);
}

#[test]
fn start_combat_null_units_do_nothing() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(21, 10).with(22, 10));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 100));
    let orig = DamageRecord {
        result: hit::result::HIT,
        ..DamageRecord::default()
    };
    let seed = f.units[a].seed;
    let mut rec = orig;
    start_combat(&mut f, &s, &c, Some(a), None, &mut rec, 128);
    start_combat(&mut f, &s, &c, None, Some(d), &mut rec, 128);
    assert_eq!(rec, orig);
    assert_eq!(f.units[a].seed, seed);
    assert!(f.units[a].combat.is_empty());
    assert!(f.log.is_empty());
}

#[test]
fn pipeline_rolls_then_applies_stored_record() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(21, 10).with(22, 10));
    let d = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(6, 100_000)
            .with(7, 100_000),
    );
    let mut rec = DamageRecord {
        result: hit::result::HIT,
        ..DamageRecord::default()
    };
    start_combat(&mut f, &s, &c, Some(a), Some(d), &mut rec, 128);
    // Rolled (2560 + roll(256)), totals done, a copy stored.
    assert!(rec.physical >= 2_560 && rec.total == rec.physical);
    assert_eq!(f.units[a].combat.len(), 1);
    assert_eq!(f.units[a].combat[0].record, rec);
    // Nothing applied yet.
    assert_eq!(f.get(d, 6), 100_000);
    // Damage frame: the stored record is applied, then freed.
    apply_melee(&mut f, &c, a, d);
    assert_eq!(f.get(d, 6), 100_000 - rec.total);
    assert!(f.units[a].combat.is_empty());
}

/// Attacker stats that make every `SrcDam`-scaled term non-zero.
fn scaled_attacker(f: &mut Fake) -> usize {
    f.add(
        FUnit::new(UnitType::Player, 0)
            .with(21, 10)
            .with(22, 20)
            .with(48, 10)
            .with(49, 20)
            .with(54, 3)
            .with(55, 6)
            .with(56, 100)
            .with(57, 512)
            .with(58, 1_024)
            .with(59, 100)
            .with(66, 40),
    )
}

#[test]
fn fill_srcdam_zero_is_128() {
    let (s, c) = (st(), ct());
    let mut f = world();
    let a = scaled_attacker(&mut f);
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let run = |srcdam: i32| {
        let mut g = f.clone();
        let mut rec = DamageRecord::default();
        fill(&mut g, &s, &c, a, d, &mut rec, false, srcdam);
        (rec, g.units[a].seed)
    };
    let (r0, s0) = run(0);
    let (r128, s128) = run(128);
    assert_eq!((r0, s0), (r128, s128));
    // Not a scale of 0, and not unscaled-by-accident: 64 differs.
    assert!(r0.physical > 0 && r0.cold_len == 100 && r0.stun_len == 40);
    assert_ne!(run(64).0, r128);
}

#[test]
fn fill_brackets_dual_wield_switch() {
    let (s, c) = (st(), ct());
    for offhand in [false, true] {
        let mut w = Spy::new();
        let a = w.f.add(FUnit::new(UnitType::Player, 0));
        let d = w.f.add(FUnit::new(UnitType::Monster, 0));
        let mut rec = DamageRecord::default();
        fill(&mut w, &s, &c, a, d, &mut rec, offhand, 128);
        // On first, the roll (including the attacked-in-melee event, event 3) inside, off last.
        assert_eq!(
            w.log(),
            [
                format!("dual {a} {offhand} true"),
                format!("event 3 {d} {a}"),
                format!("dual {a} {offhand} false"),
            ]
        );
    }
}

#[test]
fn fill_bonuses_vs_monsters() {
    let s = st();
    let mut m = monster_rec();
    m.montype = 4;
    let c = combat_tables(vec![m]);
    let mut w = Spy::new();
    let a = w.f.add(FUnit::new(UnitType::Player, 0));
    let blunt = w.f.add_item(FItem {
        types: vec![57],
        ..FItem::default()
    });
    w.f.units[a].weapon = Some(blunt);
    w.f.units[a].entries.insert(180, vec![(4, 11), (5, 13)]);
    // Stats 120–122 through the item/skill getter only.
    w.item_only.insert((a, 120), -4);
    w.item_only.insert((a, 121), 30);
    w.item_only.insert((a, 122), 7);
    let d = w.f.add(FUnit::new(UnitType::Monster, 0).with(31, 10));
    w.f.units[d].demon = true;
    w.f.units[d].undead = true;
    let mut rec = DamageRecord::default();
    fill(&mut w, &s, &c, a, d, &mut rec, false, 128);
    // Demon 30, undead 50 (blunt) + 7, montype layer 4: 11.
    assert_eq!(rec.enh_pct, 30 + 50 + 7 + 11);
    assert_eq!(w.f.get(d, 31), 6);
    assert!(w.log().contains(&format!("set {d} 31 6")));
    // Permanent: every rolled hit lowers it again, floored at 0.
    fill(
        &mut w,
        &s,
        &c,
        a,
        d,
        &mut DamageRecord::default(),
        false,
        128,
    );
    assert_eq!(w.f.get(d, 31), 2);
    fill(
        &mut w,
        &s,
        &c,
        a,
        d,
        &mut DamageRecord::default(),
        false,
        128,
    );
    assert_eq!(w.f.get(d, 31), 0);
    // Non-positive demon percent is not added; no blunt weapon: 0 + 7.
    w.item_only.insert((a, 121), -5);
    w.f.units[a].weapon = None;
    let mut rec = DamageRecord::default();
    fill(&mut w, &s, &c, a, d, &mut rec, false, 128);
    assert_eq!(rec.enh_pct, 7 + 11);
    // Player defender: none of it.
    let p = w.f.add(FUnit::new(UnitType::Player, 0).with(31, 10));
    w.f.units[p].demon = true;
    w.f.units[p].undead = true;
    let mut rec = DamageRecord::default();
    fill(&mut w, &s, &c, a, p, &mut rec, false, 128);
    assert_eq!((rec.enh_pct, w.f.get(p, 31)), (0, 10));
    // A monster attacker: only with alignment 2 (good).
    let mo = w.f.add(FUnit::new(UnitType::Monster, 0));
    w.item_only.insert((mo, 122), 7);
    let mut rec = DamageRecord::default();
    fill(&mut w, &s, &c, mo, d, &mut rec, false, 128);
    assert_eq!(rec.enh_pct, 0);
    w.alignment.insert(mo, 2);
    let mut rec = DamageRecord::default();
    fill(&mut w, &s, &c, mo, d, &mut rec, false, 128);
    assert_eq!(rec.enh_pct, 7);
}

#[test]
fn fill_cold_length_only_with_cold() {
    let (s, c) = (st(), ct());
    let mut f = world();
    // Cold 1–1 (no draw), coldlength 100, SrcDam 64: += 50.
    let a = f.add(
        FUnit::new(UnitType::Player, 0)
            .with(54, 1)
            .with(55, 1)
            .with(56, 100),
    );
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    let mut rec = DamageRecord {
        cold_len: 7,
        ..DamageRecord::default()
    };
    fill(&mut f, &s, &c, a, d, &mut rec, false, 64);
    assert_eq!((rec.cold, rec.cold_len), (128, 57));
    // No cold: the length is not touched.
    let b = f.add(FUnit::new(UnitType::Player, 0).with(56, 100));
    let mut rec = DamageRecord {
        cold_len: 7,
        ..DamageRecord::default()
    };
    fill(&mut f, &s, &c, b, d, &mut rec, false, 64);
    assert_eq!((rec.cold, rec.cold_len), (0, 7));
}

#[test]
fn bonuses_adds_damagepercent() {
    let s = st();
    let mut f = world();
    // No weapon, str 0: min 256, max 512; damagepercent 100 → 512..1024.
    let u = f.add(FUnit::new(UnitType::Player, 0).with(25, 100));
    let mut seed = f.units[u].seed;
    let v = bonuses(&mut f, &s, u, true, None, 0, 0, 0, 0, 128);
    assert_eq!(v, 512 + seed.roll(512) as i32);
    // Added to the caller's percent: 50 + 100 → 640..1280.
    let mut seed = f.units[u].seed;
    let v = bonuses(&mut f, &s, u, true, None, 0, 0, 50, 0, 128);
    assert_eq!(v, 640 + seed.roll(640) as i32);
}

#[test]
fn bonuses_reads_unit_getter() {
    let s = st();
    let stats: [(u16, i32); 7] = [
        (0, 30),
        (2, 20),
        (17, 10),
        (18, 10),
        (21, 2),
        (22, 7),
        (25, 10),
    ];
    let setup = |unit_getter: bool| {
        let mut w = Spy::new();
        let u = w.f.add(FUnit::new(UnitType::Player, 0));
        let wpn = w.f.add_item(FItem {
            str_dex: (100, 100),
            ..FItem::default()
        });
        w.f.units[u].weapon = Some(wpn);
        for (k, v) in stats {
            if unit_getter {
                w.unit_only.insert((u, k), v);
            } else {
                w.item_only.insert((u, k), v);
            }
        }
        (w, u)
    };
    // Unit getter: min 512, max 1792; pct 10 + 30 + 20 = 60; min% and
    // max% 10 each: 512 + 358 = 870, 1792 + 1254 = 3046.
    let (mut w, u) = setup(true);
    let mut seed = w.f.units[u].seed;
    let v = bonuses(&mut w, &s, u, true, None, 0, 0, 0, 0, 128);
    assert_eq!(v, 870 + seed.roll(3_046 - 870) as i32);
    // The same values in the item/skill getter are not read: 256..512.
    let (mut w, u) = setup(false);
    let mut seed = w.f.units[u].seed;
    let v = bonuses(&mut w, &s, u, true, None, 0, 0, 0, 0, 128);
    assert_eq!(v, 256 + seed.roll(256) as i32);
    // Wield type 2 reads 23 / 24 through the unit getter.
    let (mut w, u) = setup(false);
    let wpn = w.f.units[u].weapon.unwrap();
    w.f.items[wpn].wield = 2;
    w.unit_only.insert((u, 23), 2);
    w.unit_only.insert((u, 24), 7);
    w.item_only.insert((u, 23), 50);
    w.item_only.insert((u, 24), 90);
    let mut seed = w.f.units[u].seed;
    let v = bonuses(&mut w, &s, u, true, None, 0, 0, 0, 0, 128);
    assert_eq!(v, 512 + seed.roll(1_792 - 512) as i32);
}

// ================================================================ totals

/// A record field accessor (as `damage.rs`).
type Field = fn(&mut DamageRecord) -> &mut i32;

/// One resistance row: field, resist, max, pierce, absorb (%, flat), DR (0 none, 1 normal,
/// 2 magic), scaled.
type Row = (
    Field,
    u16,
    Option<u16>,
    Option<u16>,
    Option<(u16, u16)>,
    u8,
    bool,
);

/// Resistance rows 0–8.
const ROWS: [Row; 9] = [
    (|r| &mut r.physical, 36, None, None, None, 1, true),
    (
        |r| &mut r.fire,
        39,
        Some(40),
        Some(333),
        Some((142, 143)),
        2,
        true,
    ),
    (
        |r| &mut r.lightning,
        41,
        Some(42),
        Some(334),
        Some((144, 145)),
        2,
        true,
    ),
    (
        |r| &mut r.cold,
        43,
        Some(44),
        Some(335),
        Some((148, 149)),
        2,
        true,
    ),
    (
        |r| &mut r.magic,
        37,
        Some(38),
        None,
        Some((146, 147)),
        2,
        true,
    ),
    (|r| &mut r.cold_len, 43, Some(44), Some(335), None, 0, false),
    (
        |r| &mut r.freeze_len,
        43,
        Some(44),
        Some(335),
        None,
        0,
        false,
    ),
    (|r| &mut r.poison_len, 110, None, Some(336), None, 0, false),
    (|r| &mut r.poison, 45, Some(46), Some(336), None, 0, true),
];

/// Resistance rows 9–11 (the leech rows).
const LEECH_ROWS: [Field; 3] = [
    |r| &mut r.life_leech,
    |r| &mut r.mana_leech,
    |r| &mut r.stamina_leech,
];

/// Runs `totals` on a record with only `field` = 1000; returns (field,
/// absorbed).
fn one_row(f: &mut Fake, a: usize, d: usize, field: Field) -> (i32, i32) {
    let c = ct();
    let mut rec = DamageRecord::default();
    *field(&mut rec) = 1_000;
    totals(f, &c, Some(a), d, &mut rec);
    (*field(&mut rec), rec.absorbed)
}

#[test]
fn resistance_rows_table() {
    let all_pierce = [333u16, 334, 335, 336];
    let all_absorb = [142u16, 143, 144, 145, 146, 147, 148, 149];
    for (i, &(field, resist, max, pierce, absorb, dr, scaled)) in ROWS.iter().enumerate() {
        let mut f = world();
        let player = f.add(FUnit::new(UnitType::Player, 0));
        let monster = f.add(FUnit::new(UnitType::Monster, 0));
        // Resist stat on a monster defender (no penalty, no cap): 50 %.
        let d = f.add(FUnit::new(UnitType::Monster, 0).with(resist, 50));
        assert_eq!(one_row(&mut f, player, d, field).0, 500, "row {i} resist");
        // Pierce stat of the attacker; rows without one ignore them all.
        let pa = f.add(FUnit::new(UnitType::Player, 0));
        match pierce {
            Some(p) => f.set(pa, p, 20),
            None => all_pierce.iter().for_each(|&p| f.set(pa, p, 20)),
        }
        let want = if pierce.is_some() { 700 } else { 500 };
        assert_eq!(one_row(&mut f, pa, d, field).0, want, "row {i} pierce");
        // Max stat on a player defender: res 90, cap 75 + 10 = 85; rows
        // without one: 75, damage resist 50.
        let pd = f.add(FUnit::new(UnitType::Player, 0).with(resist, 90));
        if let Some(m) = max {
            f.set(pd, m, 10);
        }
        let want = match (max, resist) {
            (Some(_), _) => 150,
            (None, 36) => 500,
            (None, _) => 250,
        };
        assert_eq!(one_row(&mut f, monster, pd, field).0, want, "row {i} max");
        // Absorb stats (defender, no resist): 10 % or 1 point flat; rows
        // without them ignore every absorb stat.
        match absorb {
            Some((pc, flat)) => {
                let d1 = f.add(FUnit::new(UnitType::Monster, 0).with(pc, 10));
                assert_eq!(one_row(&mut f, player, d1, field), (900, 100), "row {i}");
                let d2 = f.add(FUnit::new(UnitType::Monster, 0).with(flat, 1));
                assert_eq!(one_row(&mut f, player, d2, field), (744, 256), "row {i}");
            }
            None => {
                let d1 = f.add(FUnit::new(UnitType::Monster, 0));
                all_absorb.iter().for_each(|&s| f.set(d1, s, 10));
                assert_eq!(one_row(&mut f, player, d1, field), (1_000, 0), "row {i}");
            }
        }
        // Damage reduction: normal (34) and magic (35), 1 point each.
        let dn = f.add(FUnit::new(UnitType::Monster, 0).with(34, 1));
        let dm = f.add(FUnit::new(UnitType::Monster, 0).with(35, 1));
        let (wn, wm) = match dr {
            1 => (744, 1_000),
            2 => (1_000, 744),
            _ => (1_000, 1_000),
        };
        assert_eq!(one_row(&mut f, player, dn, field).0, wn, "row {i} dr");
        assert_eq!(one_row(&mut f, player, dm, field).0, wm, "row {i} dr");
        // Scaled by the PvP damage percent 17.
        let p2 = f.add(FUnit::new(UnitType::Player, 0));
        let want = if scaled { 170 } else { 1_000 };
        assert_eq!(one_row(&mut f, player, p2, field).0, want, "row {i} scaled");
    }
    for (i, &field) in LEECH_ROWS.iter().enumerate() {
        let mut f = world();
        let player = f.add(FUnit::new(UnitType::Player, 0));
        let monster = f.add(FUnit::new(UnitType::Monster, 0));
        // Scaled (PvP 17 %), then not resisted (player attacker).
        let p2 = f.add(FUnit::new(UnitType::Player, 0));
        assert_eq!(one_row(&mut f, player, p2, field).0, 170, "leech {i}");
        // Monster attacker: the row runs with no resist, no DR, no absorb.
        let d = f.add(FUnit::new(UnitType::Player, 0).with(34, 1).with(35, 1));
        for s in [36u16, 39, 41, 43, 45, 37, 110] {
            f.set(d, s, 50);
        }
        all_absorb.iter().for_each(|&s| f.set(d, s, 10));
        assert_eq!(one_row(&mut f, monster, d, field), (1_000, 0), "leech {i}");
    }
}

#[test]
fn att_mon_and_def_mon_exclude_hirelings() {
    let c = ct();
    let mut f = world();
    let p = f.add(FUnit::new(UnitType::Player, 0));
    // def_mon: a hireling defender takes the player caps (75), a plain
    // monster none.
    let h = f.add(FUnit::new(UnitType::Monster, 0).with(39, 90));
    f.units[h].hireling = true;
    let m = f.add(FUnit::new(UnitType::Monster, 0).with(39, 90));
    let fire = |f: &mut Fake, d| {
        let mut rec = DamageRecord {
            fire: 1_000,
            ..DamageRecord::default()
        };
        totals(f, &c, Some(p), d, &mut rec);
        rec.fire
    };
    assert_eq!(fire(&mut f, h), 250);
    assert_eq!(fire(&mut f, m), 100);
    // att_mon: leech rows run (v ≤ 0 → 0) and life leech joins the total
    // only for a non-hireling monster attacker; a hireling stops at the
    // first leech row.
    let ha = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[ha].hireling = true;
    let ma = f.add(FUnit::new(UnitType::Monster, 0));
    let run = |f: &mut Fake, a| {
        let mut rec = DamageRecord {
            physical: 1_000,
            life_leech: 300,
            mana_leech: -5,
            stamina_leech: -7,
            ..DamageRecord::default()
        };
        totals(f, &c, Some(a), m, &mut rec);
        (rec.mana_leech, rec.stamina_leech, rec.total)
    };
    assert_eq!(run(&mut f, ha), (-5, -7, 1_000));
    assert_eq!(run(&mut f, ma), (0, 0, 1_300));
}

#[test]
fn row_non_positive_and_negative_after_dr() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    // Fire ≤ 0 → 0, nothing absorbed.
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(142, 40).with(143, 5));
    let mut rec = DamageRecord {
        fire: -100,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!((rec.fire, rec.absorbed), (0, 0));
    // Physical 100 − DR 256 = −156: kept negative, resist (50) not
    // applied, summed as is.
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(34, 1).with(36, 50));
    let mut rec = DamageRecord {
        physical: 100,
        fire: 400,
        ..DamageRecord::default()
    };
    totals(&mut f, &c, Some(a), d, &mut rec);
    assert_eq!((rec.physical, rec.total), (-156, 400 - 156));
}

// ================================================================ application

/// Player attacker and monster defender in a `Spy`.
fn spy_pair() -> (Spy, usize, usize) {
    let mut w = Spy::new();
    let a = w.f.add(FUnit::new(UnitType::Player, 0));
    let d = w.f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(6, 100_000)
            .with(7, 100_000),
    );
    (w, a, d)
}

fn entry(w: &Spy, a: usize, d: usize, record: DamageRecord) -> CombatEntry {
    CombatEntry {
        attacker: w.ident(a),
        defender: w.ident(d),
        record,
    }
}

#[test]
fn apply_melee_out_of_range_frees_and_returns() {
    let c = ct();
    let (mut w, a, d) = spy_pair();
    let d2 = w.f.add(FUnit::new(UnitType::Monster, 0));
    let hit = DamageRecord {
        result: hit::result::HIT,
        total: 500,
        ..DamageRecord::default()
    };
    let list = vec![
        entry(&w, a, d, hit),
        entry(&w, a, d2, hit),
        entry(&w, a, d, hit),
    ];
    w.f.units[a].combat = list.clone();
    w.f.in_range = false;
    apply_melee(&mut w, &c, a, d);
    // Player attacker: range 1; only this defender's records go.
    assert_eq!(*w.ranges.borrow(), [(a, d, 1)]);
    assert_eq!(w.f.units[a].combat, [list[1].clone()]);
    assert!(w.log().is_empty());
    assert_eq!(w.f.get(d, 6), 100_000);
    // Monster attacker: range 3.
    let ma = w.f.add(FUnit::new(UnitType::Monster, 0));
    w.f.units[ma].combat = vec![entry(&w, ma, d, hit)];
    apply_melee(&mut w, &c, ma, d);
    assert_eq!(w.ranges.borrow()[1], (ma, d, 3));
    assert!(w.f.units[ma].combat.is_empty());
}

#[test]
fn apply_melee_hit_steps() {
    let c = ct();
    // A player attacker in mode 0 (dead) returns at once; the combat record is not
    // freed.
    let (mut w, a, d) = spy_pair();
    let hit = DamageRecord {
        result: hit::result::HIT,
        total: 500,
        ..DamageRecord::default()
    };
    w.f.units[a].combat = vec![entry(&w, a, d, hit)];
    w.f.units[a].mode = 0;
    apply_melee(&mut w, &c, a, d);
    assert_eq!(w.f.units[a].combat.len(), 1);
    assert!(w.log().is_empty());
    assert_eq!(w.f.get(d, 6), 100_000);
    // Hit flags are replaced by 0x20, so a stored "ignore hostility" is dropped: not
    // hostile → apply stops (no damage, no melee events).
    let (mut w, a, d) = spy_pair();
    w.f.hostile = false;
    let rec = DamageRecord {
        hit_flags: hitflag::IGNORE_HOSTILITY,
        ..hit
    };
    w.f.units[a].combat = vec![entry(&w, a, d, rec)];
    apply_melee(&mut w, &c, a, d);
    assert_eq!(w.f.get(d, 6), 100_000);
    assert!(!w.log().iter().any(|l| l.starts_with("event 5")));
    // On a hostile hit: melee apply (events 5, 1), overlay, hit class from the weapon
    // (fake: 2), durability of the weapon.
    let (mut w, a, d) = spy_pair();
    let wpn = w.f.add_item(FItem {
        types: vec![45],
        durability: true,
        ..FItem::default()
    });
    w.f.units[a].weapon = Some(wpn);
    w.f.units[a].seed = seed_giving(3);
    let rec = DamageRecord { overlay: 77, ..hit };
    w.f.units[a].combat = vec![entry(&w, a, d, rec)];
    apply_melee(&mut w, &c, a, d);
    assert_eq!(w.f.get(d, 6), 100_000 - 500);
    let pos = |s: String| w.log().iter().position(|l| *l == s).unwrap();
    let e5 = pos(format!("event 5 {a} {d}"));
    let e1 = pos(format!("event 1 {d} {a}"));
    let ov = pos(format!("overlay {d} 77"));
    let dur = pos(format!("durability {a} {wpn}"));
    assert!(e5 < e1 && e1 < ov && ov < dur);
    let last = w.log().last().unwrap();
    assert!(
        last.starts_with(&format!("reaction {a} {d} 0x2 ")),
        "{last}"
    );
    // Hit class: an element nibble alone gets the weapon class; a set low
    // nibble or a fixed class does not.
    for (class, fixed, want) in [(0x30u32, 0u8, 0x32u32), (0x05, 0, 0x05), (0, 1, 0)] {
        let (mut w, a, d) = spy_pair();
        let rec = DamageRecord {
            hit_class: class,
            hit_class_fixed: fixed,
            ..hit
        };
        w.f.units[a].combat = vec![entry(&w, a, d, rec)];
        apply_melee(&mut w, &c, a, d);
        let last = w.log().last().unwrap();
        assert!(
            last.starts_with(&format!("reaction {a} {d} {want:#x} ")),
            "{last}"
        );
    }
    // No hit: no apply, no durability draw.
    let (mut w, a, d) = spy_pair();
    let wpn = w.f.add_item(FItem {
        types: vec![45],
        durability: true,
        ..FItem::default()
    });
    w.f.units[a].weapon = Some(wpn);
    let seed = w.f.units[a].seed;
    w.f.units[a].combat = vec![entry(&w, a, d, DamageRecord::default())];
    apply_melee(&mut w, &c, a, d);
    assert_eq!(w.f.units[a].seed, seed);
    assert!(!w.log().iter().any(|l| l.starts_with("last")));
}

#[test]
fn apply_melee_events_and_thorns() {
    let c = ct();
    let hit = DamageRecord {
        result: hit::result::HIT,
        ..DamageRecord::default()
    };
    for (rec, thorns) in [(hit, true), (DamageRecord::default(), false)] {
        let (mut w, a, d) = spy_pair();
        w.f.units[a].combat = vec![entry(&w, a, d, rec)];
        apply_melee(&mut w, &c, a, d);
        let log = w.log();
        let at = |s: String| log.iter().position(|l| *l == s);
        let e7 = at(format!("event 7 {a} {d}")).unwrap();
        let e3 = at(format!("event 3 {d} {a}")).unwrap();
        let th = at(format!("thorns {a} {d}"));
        let re = log.iter().position(|l| l.starts_with("reaction")).unwrap();
        assert!(e7 < e3 && e3 < re);
        assert_eq!(th.is_some(), thorns);
        if let Some(t) = th {
            assert!(e3 < t && t < re);
        }
    }
    // An object attacker in mode 0 still deals thorns on a hit.
    let (mut w, _, d) = spy_pair();
    let o = w.f.add(FUnit::new(UnitType::Object, 0));
    w.f.units[o].mode = 0;
    w.f.units[o].combat = vec![entry(&w, o, d, hit)];
    apply_melee(&mut w, &c, o, d);
    assert!(w.log().contains(&format!("thorns {o} {d}")));
}

#[test]
fn apply_skips_unkillable_and_dead() {
    let mut unkillable = monster_rec();
    unkillable.killable = false;
    let c = combat_tables(vec![monster_rec(), unkillable]);
    let rec = DamageRecord {
        result: hit::result::HIT,
        total: 500,
        ..DamageRecord::default()
    };
    // Not killable: return before anything.
    let (mut w, a, d) = spy_pair();
    w.f.units[d].class = 1;
    let mut r = rec;
    apply(&mut w, &c, a, d, false, &mut r);
    assert_eq!((w.f.get(d, 6), r), (100_000, rec));
    assert!(w.log().is_empty());
    // Dead: same.
    let (mut w, a, d) = spy_pair();
    w.dead.push(d);
    let mut r = rec;
    apply(&mut w, &c, a, d, false, &mut r);
    assert_eq!((w.f.get(d, 6), r), (100_000, rec));
    assert!(w.log().is_empty());
    // Otherwise the attacker is remembered first.
    let (mut w, a, d) = spy_pair();
    let mut r = rec;
    apply(&mut w, &c, a, d, false, &mut r);
    assert_eq!(w.log()[0], format!("last {d} {a}"));
    assert_eq!(w.f.get(d, 6), 100_000 - 500);
}

#[test]
fn apply_missile_totals_and_events() {
    let c = ct();
    let base = DamageRecord {
        result: hit::result::HIT,
        hit_flags: hitflag::ROLLED,
        fire: 1_000,
        ..DamageRecord::default()
    };
    let events = |w: &Spy| -> Vec<String> {
        w.log()
            .iter()
            .filter(|l| l.starts_with("event"))
            .cloned()
            .collect()
    };
    // Missile: totals run (fire res 50 → 500), events 11, 6, 2.
    let (mut w, a, d) = spy_pair();
    w.f.set(d, 39, 50);
    let mut r = base;
    apply(&mut w, &c, a, d, true, &mut r);
    assert_eq!((r.fire, r.total), (500, 500));
    assert_eq!(w.f.get(d, 6), 100_000 - 500);
    assert_eq!(
        events(&w),
        [
            format!("event 11 {d} {a}"),
            format!("event 6 {a} {d}"),
            format!("event 2 {d} {a}")
        ]
    );
    // No event 6 with hit flag 0x80, or without 0x20; event 2 stays.
    for flags in [hitflag::ROLLED | hitflag::NO_MISSILE_EVENT, 0] {
        let (mut w, a, d) = spy_pair();
        let mut r = DamageRecord {
            hit_flags: flags,
            ..base
        };
        apply(&mut w, &c, a, d, true, &mut r);
        assert_eq!(
            events(&w),
            [format!("event 11 {d} {a}"), format!("event 2 {d} {a}")]
        );
    }
    // Melee: no totals (total stays 0, no event 11), events 5 then 1.
    let (mut w, a, d) = spy_pair();
    let mut r = base;
    apply(&mut w, &c, a, d, false, &mut r);
    assert_eq!((r.fire, r.total, w.f.get(d, 6)), (1_000, 0, 100_000));
    assert_eq!(
        events(&w),
        [format!("event 5 {a} {d}"), format!("event 1 {d} {a}")]
    );
    // "No events" result, or no hit: none of them.
    for result in [hit::result::HIT | hit::result::NO_EVENTS, 0] {
        let (mut w, a, d) = spy_pair();
        let mut r = DamageRecord { result, ..base };
        apply(&mut w, &c, a, d, false, &mut r);
        assert!(events(&w).is_empty());
    }
}

#[test]
fn apply_caps_physical_before_leech() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(6, 0).with(7, 100_000));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 1_280));
    let mut rec = DamageRecord {
        result: hit::result::HIT,
        physical: 2_560,
        total: 2_560,
        life_leech: 5,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, false, &mut rec);
    // Leech on the capped 1280: pct(1280, 320, 100) / 64 = 64 (not 128).
    assert_eq!(rec.physical, 1_280);
    assert_eq!(f.get(a, 6), 64);
    assert_eq!(rec.life_leech, 320);
}

#[test]
fn apply_monster_hit_hook_on_hit_only() {
    let c = ct();
    for (result, want) in [(hit::result::HIT, true), (0, false)] {
        let mut f = world();
        let a = f.add(FUnit::new(UnitType::Monster, 0));
        let d = f.add(FUnit::new(UnitType::Player, 0).with(6, 10_000));
        let mut rec = DamageRecord {
            result,
            ..DamageRecord::default()
        };
        apply(&mut f, &c, a, d, false, &mut rec);
        assert_eq!(f.log.contains(&format!("monhit {a}")), want);
    }
    // A player attacker never.
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 10_000));
    let mut rec = DamageRecord {
        result: hit::result::HIT,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, false, &mut rec);
    assert!(!f.log.iter().any(|l| l.starts_with("monhit")));
}

#[test]
fn apply_heals_absorbed_before_damage() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    // Life at max: the heal adds nothing, then 700 is taken (300 left;
    // damage first would leave 800).
    let d = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(6, 1_000)
            .with(7, 1_000),
    );
    let mut rec = DamageRecord {
        absorbed: 500,
        total: 700,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, false, &mut rec);
    assert_eq!(f.get(d, 6), 300);
    // With room: + 300, no damage.
    f.set(d, 6, 500);
    let mut rec = DamageRecord {
        absorbed: 300,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, false, &mut rec);
    assert_eq!(f.get(d, 6), 800);
}

#[test]
fn apply_drains_defender_mana_and_stamina() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0).with(9, 100_000));
    let d = f.add(
        FUnit::new(UnitType::Monster, 0)
            .with(6, 100_000)
            .with(8, 1_000)
            .with(10, 1_000),
    );
    let rec = DamageRecord {
        result: hit::result::HIT,
        physical: 2_560,
        mana_leech: 5,
        stamina_leech: 100,
        ..DamageRecord::default()
    };
    // 5 % mana steal on Normal removes 5 << 6 = 320.
    let mut r = rec;
    apply(&mut f, &c, a, d, false, &mut r);
    assert_eq!((f.get(d, 8), f.get(d, 10)), (680, 900));
    // Nightmare divisor 2: 160.
    f.difficulty = 1;
    f.set(d, 8, 1_000);
    let mut r = rec;
    apply(&mut f, &c, a, d, false, &mut r);
    assert_eq!(f.get(d, 8), 840);
    // Below 256 after the drain: 0.
    f.difficulty = 0;
    f.set(d, 8, 500);
    f.set(d, 10, 300);
    let mut r = rec;
    apply(&mut f, &c, a, d, false, &mut r);
    assert_eq!((f.get(d, 8), f.get(d, 10)), (0, 0));
}

#[test]
fn apply_timed_effects_in_order() {
    let mut m = monster_rec();
    m.coldeffect = (-50i8) as u8;
    let c = combat_tables(vec![m]);
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0).with(6, 100_000));
    let mut rec = DamageRecord {
        stun_len: 10,
        cold_len: 20,
        freeze_len: 30,
        poison: 100,
        poison_len: 40,
        burn: 100,
        burn_len: 50,
        ..DamageRecord::default()
    };
    apply(&mut f, &c, a, d, false, &mut rec);
    let lists: Vec<&str> = f
        .log
        .iter()
        .filter(|l| l.starts_with("list "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        lists,
        [
            "list 1 21 0 10",
            "list 1 11 0 20",
            "list 1 1 0 30",
            "list 1 2 0 40",
            "list 1 115 0 50"
        ]
    );
}

#[test]
fn stun_non_positive_length_does_nothing() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[d].flags = 8;
    let seed = f.units[a].seed;
    stun(&mut f, &c, a, d, 0);
    stun(&mut f, &c, a, d, -3);
    assert!(f.log.is_empty());
    assert_eq!(f.units[a].seed, seed);
}

#[test]
fn cold_non_positive_length_does_nothing() {
    let c = ct();
    let mut f = world();
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Player, 0));
    let seed = f.units[d].seed;
    cold(&mut f, &c, a, d, 0);
    cold(&mut f, &c, a, d, -3);
    assert!(f.log.is_empty());
    assert_eq!(f.units[d].seed, seed);
}

// ================================================================ get-hit and durability

#[test]
fn get_hit_freeze_mask2_and_mode() {
    let c = ct();
    let mut w = Spy::new();
    let p = w.f.add(FUnit::new(UnitType::Player, 0).with(7, 25_600));
    // Frozen: true, before any size test or draw.
    w.f.units[p].states.push(1);
    let big = DamageRecord {
        total: 100_000,
        ..DamageRecord::default()
    };
    let seed = w.f.units[p].seed;
    assert!(no_get_hit(&mut w, &c.hitclass, p, &big, 1));
    assert_eq!(w.f.units[p].seed, seed);
    w.f.units[p].states.clear();
    // hth (div 16), total 2000: ≥ 1600, < 3200 → mask(2). 0 → true after
    // one draw; otherwise mask(4) (< 6400), here non-zero → false.
    let rec = DamageRecord {
        total: 2_000,
        ..DamageRecord::default()
    };
    w.f.units[p].seed = seed_giving(0);
    assert!(no_get_hit(&mut w, &c.hitclass, p, &rec, 1));
    assert_eq!(w.f.units[p].seed, Seed::new(0, 0));
    w.f.units[p].seed = seed_giving(1);
    let mut s = seed_giving(1);
    assert_eq!(s.mask(2), 1);
    assert_ne!(s.mask(4), 0);
    assert!(!no_get_hit(&mut w, &c.hitclass, p, &rec, 1));
    assert_eq!(w.f.units[p].seed, s);
    // Total ≥ M / 4: no draws; a player or a monster with `GH` enters
    // get-hit (false), a monster without mode 3 does not (true).
    let rec = DamageRecord {
        total: 10_000,
        ..DamageRecord::default()
    };
    let seed = w.f.units[p].seed;
    assert!(!no_get_hit(&mut w, &c.hitclass, p, &rec, 1));
    assert_eq!(w.f.units[p].seed, seed);
    let m = w.f.add(FUnit::new(UnitType::Monster, 0).with(7, 25_600));
    assert!(!no_get_hit(&mut w, &c.hitclass, m, &rec, 1));
    w.no_mode.push((m, 4));
    assert!(!no_get_hit(&mut w, &c.hitclass, m, &rec, 1));
    w.no_mode.push((m, 3));
    assert!(no_get_hit(&mut w, &c.hitclass, m, &rec, 1));
    // The mode test is for monsters only.
    w.no_mode.push((p, 3));
    assert!(!no_get_hit(&mut w, &c.hitclass, p, &rec, 1));
}

#[test]
fn durability_attacker_weapon() {
    let mut f = world();
    let wpn = f.add_item(FItem {
        types: vec![45],
        durability: true,
        ..FItem::default()
    });
    let a = f.add(FUnit::new(UnitType::Player, 0));
    let d = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[a].weapon = Some(wpn);
    // Player with a weapon: durability_hit on it (4 %, attacker seed).
    f.units[a].seed = seed_giving(3);
    durability(&mut f, a, d);
    assert_eq!(f.log, [format!("durability {a} {wpn}")]);
    f.units[a].seed = seed_giving(4);
    durability(&mut f, a, d);
    assert_eq!(f.log.len(), 1);
    assert_eq!(f.units[a].seed, Seed::new(4, 0));
    // No weapon, or a monster attacker with one: no draw.
    let m = f.add(FUnit::new(UnitType::Monster, 0));
    f.units[m].weapon = Some(wpn);
    f.units[a].weapon = None;
    let (sa, sm) = (f.units[a].seed, f.units[m].seed);
    durability(&mut f, a, d);
    durability(&mut f, m, d);
    assert_eq!((f.units[a].seed, f.units[m].seed), (sa, sm));
    assert_eq!(f.log.len(), 1);
}
