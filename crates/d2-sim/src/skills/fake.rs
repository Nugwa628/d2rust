//! A small fake of the [`SkillUnits`] / [`CombatWorld`] seams for unit
//! tests: units are indices into a `Vec`, stats a `BTreeMap`, every
//! effect is logged.

use super::{KickItems, LearnUnits, ManaUnits, SkillEntry, SkillTables, SkillUnits};
use crate::combat::{CombatEntry, CombatTables, CombatWorld, DamageRecord, RoomKind};
use crate::rng::Seed;
use crate::units::UnitType;
use d2_data::tables::{
    Charstats, Difficultylevels, Missiles, Monstats, Monstats2, Record, Skilldesc, Skills,
};
use std::collections::BTreeMap;

/// One fake unit.
#[derive(Debug, Clone)]
pub struct FUnit {
    pub kind: UnitType,
    pub class: i32,
    pub stats: BTreeMap<(u16, u16), i32>,
    pub base: BTreeMap<(u16, u16), i32>,
    pub entries: BTreeMap<u16, Vec<(u16, i32)>>,
    pub states: Vec<u16>,
    /// (state, stat) → value of state stat lists.
    pub state_stats: BTreeMap<(u16, u16), i32>,
    pub state_expiry: BTreeMap<u16, i32>,
    pub seed: Seed,
    pub skills: Vec<SkillEntry>,
    pub used: Option<SkillEntry>,
    pub weapon: Option<usize>,
    pub items: BTreeMap<u8, usize>,
    pub mode: i32,
    pub moving: bool,
    pub flags: u32,
    pub boss: bool,
    pub hireling: bool,
    pub demon: bool,
    pub undead: bool,
    pub revived: bool,
    pub prime_evil: bool,
    pub shield: bool,
    pub weapon_class: i32,
    pub combat: Vec<CombatEntry>,
    pub room: RoomKind,
    pub guid: u32,
    /// Monster type flags (`monster_flag`); `None` reads `flags`.
    pub type_flags: Option<u32>,
    pub align: i32,
    pub no_walk: bool,
}

impl FUnit {
    pub fn new(kind: UnitType, class: i32) -> Self {
        Self {
            kind,
            class,
            stats: BTreeMap::new(),
            base: BTreeMap::new(),
            entries: BTreeMap::new(),
            states: Vec::new(),
            state_stats: BTreeMap::new(),
            state_expiry: BTreeMap::new(),
            seed: Seed::new(1, 0),
            skills: Vec::new(),
            used: None,
            weapon: None,
            items: BTreeMap::new(),
            mode: 1,
            moving: false,
            flags: 0,
            boss: false,
            hireling: false,
            demon: false,
            undead: false,
            revived: false,
            prime_evil: false,
            shield: false,
            weapon_class: 0,
            combat: Vec::new(),
            room: RoomKind::Field,
            guid: 0,
            type_flags: None,
            align: 0,
            no_walk: false,
        }
    }

    pub fn with(mut self, stat: u16, v: i32) -> Self {
        self.stats.insert((stat, 0), v);
        self
    }
}

/// One fake item.
#[derive(Debug, Clone, Default)]
pub struct FItem {
    pub types: Vec<i32>,
    pub wield: i32,
    pub damage: (i32, i32),
    pub str_dex: (i32, i32),
    pub throw: bool,
    pub durability: bool,
    pub boots: (i32, i32),
}

/// The fake world; `log` records every effect in call order.
#[derive(Debug, Clone, Default)]
pub struct Fake {
    pub units: Vec<FUnit>,
    pub items: Vec<FItem>,
    pub frame: i32,
    pub expansion: bool,
    pub difficulty: usize,
    pub counter: u8,
    pub hostile: bool,
    pub in_range: bool,
    /// `Some(r)`: in melee range only for range argument `r`.
    pub range_needed: Option<i32>,
    /// Every unit is shapeshifted (`ManaUnits::shapeshifted`).
    pub shifted: bool,
    /// The record `reaction` was last called with.
    pub last_reaction: Option<DamageRecord>,
    /// Units `is_dead` reports as dead.
    pub dead_units: std::collections::BTreeSet<usize>,
    /// Units' alignment (default 0).
    pub aligned: BTreeMap<usize, i32>,
    pub log: Vec<String>,
    /// The (attacker, defender, record) of every `reaction` call.
    pub reactions: Vec<(usize, usize, DamageRecord)>,
    /// Per-unit override of the monster type flags (`monster_flag`); a
    /// unit without an entry uses its `flags`.
    pub mflags: BTreeMap<usize, u32>,
    /// The rules the formulas read (default D2RUST's own).
    pub rules: crate::rules::Ruleset,
}

impl Fake {
    pub fn add(&mut self, mut u: FUnit) -> usize {
        u.guid = self.units.len() as u32;
        self.units.push(u);
        self.units.len() - 1
    }
    pub fn add_item(&mut self, i: FItem) -> usize {
        self.items.push(i);
        self.items.len() - 1
    }
    pub fn set(&mut self, u: usize, stat: u16, v: i32) {
        self.units[u].stats.insert((stat, 0), v);
    }
    pub fn get(&self, u: usize, stat: u16) -> i32 {
        self.units[u].stats.get(&(stat, 0)).copied().unwrap_or(0)
    }
}

impl SkillUnits for Fake {
    type Unit = usize;
    type Item = usize;
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    fn unit_type(&self, u: usize) -> UnitType {
        self.units[u].kind
    }
    fn class_id(&self, u: usize) -> i32 {
        self.units[u].class
    }
    fn stat(&self, u: usize, stat: u16, layer: u16) -> i32 {
        self.units[u]
            .stats
            .get(&(stat, layer))
            .copied()
            .unwrap_or(0)
    }
    fn item_stat(&self, u: usize, stat: u16, layer: u16) -> i32 {
        self.stat(u, stat, layer)
    }
    fn base_stat(&self, u: usize, stat: u16, layer: u16) -> i32 {
        self.units[u].base.get(&(stat, layer)).copied().unwrap_or(0)
    }
    fn formula_stat(&self, u: usize, stat: u16, _mode: i32) -> i32 {
        self.stat(u, stat, 0)
    }
    fn stat_entries(&self, u: usize, stat: u16, max: usize) -> Vec<(u16, i32)> {
        let mut v = self.units[u]
            .entries
            .get(&stat)
            .cloned()
            .unwrap_or_default();
        v.truncate(max);
        v
    }
    fn has_state(&self, u: usize, state: u16) -> bool {
        self.units[u].states.contains(&state)
    }
    fn state_stat(&self, u: usize, state: u16, stat: u16) -> Option<i32> {
        self.units[u].state_stats.get(&(state, stat)).copied()
    }
    fn seed(&mut self, u: usize) -> &mut Seed {
        &mut self.units[u].seed
    }
    fn skill_list(&self, u: usize) -> Vec<SkillEntry> {
        self.units[u].skills.clone()
    }
    fn used_skill(&self, u: usize) -> Option<SkillEntry> {
        self.units[u].used
    }
    fn current_weapon(&self, u: usize) -> Option<usize> {
        self.units[u].weapon
    }
    fn weapon(&self, u: usize) -> Option<usize> {
        self.units[u].weapon
    }
    fn item_at(&self, u: usize, loc: u8) -> Option<usize> {
        self.units[u].items.get(&loc).copied()
    }
    fn item_is(&self, item: usize, itype: i32) -> bool {
        self.items[item].types.contains(&itype)
    }
    fn itype_is(&self, itype: i32, parent: i32) -> bool {
        itype == parent
    }
    fn wield_type(&self, item: usize) -> i32 {
        self.items[item].wield
    }
    fn item_damage(&self, item: usize, max: bool) -> i32 {
        let d = self.items[item].damage;
        if max {
            d.1
        } else {
            d.0
        }
    }
    fn str_dex_bonus(&self, item: usize) -> (i32, i32) {
        self.items[item].str_dex
    }
    fn item_flag_throw(&self, item: usize) -> bool {
        self.items[item].throw
    }
    fn missile_level(&self, _u: usize) -> i32 {
        0
    }
}

impl CombatWorld for Fake {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn difficulty(&self) -> usize {
        self.difficulty
    }
    fn hit_class_counter(&mut self) -> &mut u8 {
        &mut self.counter
    }
    fn ident(&self, u: usize) -> (UnitType, u32) {
        (self.units[u].kind, self.units[u].guid)
    }
    fn mode(&self, u: usize) -> i32 {
        self.units[u].mode
    }
    fn moving_mode(&self, u: usize) -> bool {
        self.units[u].moving
    }
    fn monster_flag(&self, u: usize, mask: u32) -> bool {
        self.units[u]
            .type_flags
            .or_else(|| self.mflags.get(&u).copied())
            .unwrap_or(self.units[u].flags)
            & mask
            != 0
    }
    fn is_boss(&self, u: usize) -> bool {
        self.units[u].boss
    }
    fn is_hireling(&self, u: usize) -> bool {
        self.units[u].hireling
    }
    fn is_demon(&self, u: usize) -> bool {
        self.units[u].demon
    }
    fn is_undead(&self, u: usize) -> bool {
        self.units[u].undead
    }
    fn is_prime_evil(&self, u: usize) -> bool {
        self.units[u].prime_evil
    }
    fn is_revived(&self, u: usize) -> bool {
        self.units[u].revived
    }
    fn alignment(&self, u: usize) -> i32 {
        self.aligned.get(&u).copied().unwrap_or(self.units[u].align)
    }
    fn hostile(&self, _a: usize, _d: usize) -> bool {
        self.hostile
    }
    fn melee_range(&self, _u: usize) -> i32 {
        0
    }
    fn in_melee_range(&self, _a: usize, _d: usize, range: i32) -> bool {
        self.in_range && self.range_needed.is_none_or(|r| r == range)
    }
    fn has_shield(&self, u: usize) -> bool {
        self.units[u].shield
    }
    fn composit_shield(&self, u: usize) -> bool {
        self.units[u].shield
    }
    fn weapon_class(&self, u: usize) -> i32 {
        self.units[u].weapon_class
    }
    fn weapon_hit_class(&self, _u: usize) -> u32 {
        2
    }
    fn montype_matches(&self, layer: u16, montype: i32) -> bool {
        i32::from(layer) == montype
    }
    fn room(&self, u: usize) -> RoomKind {
        self.units[u].room
    }
    fn is_dead(&self, u: usize) -> bool {
        self.dead_units.contains(&u)
    }
    fn monster_has_mode(&self, u: usize, _mode: i32) -> bool {
        !self.units[u].no_walk
    }
    fn converted_type(&self, u: usize) -> i32 {
        self.units[u].kind as i32
    }
    fn item_has_durability(&self, item: usize) -> bool {
        self.items[item].durability
    }
    fn player_count_bonus(&self, players: i32) -> i32 {
        (players - 1) * 50
    }
    fn set_stat(&mut self, u: usize, stat: u16, value: i32) {
        self.log.push(format!("set {u} {stat} {value}"));
        self.units[u].stats.insert((stat, 0), value);
    }
    fn unit_event(&mut self, event: u8, unit: usize, other: usize, _r: &mut DamageRecord) {
        self.log.push(format!("event {event} {unit} {other}"));
    }
    fn set_state(&mut self, u: usize, state: u16, on: bool) {
        self.log.push(format!("state {u} {state} {on}"));
        self.units[u].states.retain(|&s| s != state);
        if on {
            self.units[u].states.push(state);
        }
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
        self.log.push(format!(
            "curse {t} {o} {state} {stat} {value} {frames} {skill} {level}"
        ));
    }
    fn state_list_expiry(&self, u: usize, state: u16) -> Option<i32> {
        self.units[u].state_expiry.get(&state).copied()
    }
    fn set_state_list_expiry(&mut self, u: usize, state: u16, e: i32) {
        self.log.push(format!("expiry {u} {state} {e}"));
        self.units[u].state_expiry.insert(state, e);
    }
    fn create_state_list(&mut self, u: usize, state: u16, owner: usize, e: i32) {
        self.log.push(format!("list {u} {state} {owner} {e}"));
        self.units[u].state_expiry.insert(state, e);
    }
    fn set_state_list_stat(&mut self, u: usize, state: u16, stat: u16, v: i32) {
        self.log.push(format!("liststat {u} {state} {stat} {v}"));
        self.units[u].state_stats.insert((state, stat), v);
    }
    fn schedule_timer(&mut self, u: usize, ty: u8, frame: i32) {
        self.log.push(format!("timer {u} {ty} {frame}"));
    }
    fn cancel_timers(&mut self, u: usize, ty: u8) {
        self.log.push(format!("cancel {u} {ty}"));
    }
    fn overlay(&mut self, u: usize, id: i32) {
        self.log.push(format!("overlay {u} {id}"));
    }
    fn refresh_anim_rate(&mut self, u: usize) {
        self.log.push(format!("anim {u}"));
    }
    fn set_last_attacker(&mut self, _d: usize, _a: usize) {}
    fn monster_hit_hook(&mut self, a: usize) {
        self.log.push(format!("monhit {a}"));
    }
    fn monster_damaged_hook(&mut self, d: usize) {
        self.log.push(format!("mondamaged {d}"));
    }
    fn dual_wield_switch(&mut self, _a: usize, _offhand: bool, _on: bool) {}
    fn combat_list(&mut self, u: usize) -> &mut Vec<CombatEntry> {
        &mut self.units[u].combat
    }
    fn durability_loss(&mut self, owner: usize, item: usize) {
        self.log.push(format!("durability {owner} {item}"));
    }
    fn thorns(&mut self, a: usize, d: usize, _r: &mut DamageRecord) {
        self.log.push(format!("thorns {a} {d}"));
    }
    fn reaction(&mut self, a: usize, d: usize, r: &mut DamageRecord) {
        self.last_reaction = Some(*r);
        self.log.push(format!("reaction {a} {d}"));
        self.reactions.push((a, d, *r));
    }
}

impl ManaUnits for Fake {
    fn shapeshifted(&self, _u: usize) -> bool {
        self.shifted
    }
    fn consume_charges(&mut self, _u: usize, _e: &SkillEntry) -> bool {
        self.log.push("charges".into());
        true
    }
    fn pay_life(&mut self, _u: usize, cost: i32) -> bool {
        self.log.push(format!("paylife {cost}"));
        true
    }
    fn set_stat(&mut self, u: usize, stat: u16, value: i32) {
        CombatWorld::set_stat(self, u, stat, value);
    }
}

impl LearnUnits for Fake {
    fn is_class_skill(&self, _u: usize, _skill: i32) -> bool {
        true
    }
    fn add_skill_level(&mut self, u: usize, skill: i32, cost: i32) {
        self.log.push(format!("addskill {u} {skill} {cost}"));
    }
    fn send_attack_reset(&mut self, u: usize) {
        self.log.push(format!("msg21 {u}"));
    }
    fn point_client_updates(&mut self, u: usize) {
        self.log.push(format!("pointupdates {u}"));
    }
}

impl KickItems for Fake {
    fn toggle_weapon_lists(&mut self, _u: usize, on: bool) {
        self.log.push(format!("weaponlists {on}"));
    }
    fn boots_damage(&self, item: usize) -> (i32, i32) {
        self.items[item].boots
    }
}

/// A zeroed record of `T` with every calc field set to "no formula" by
/// the caller as needed.
pub fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// A skills record with no formulas.
pub fn skill_rec() -> Skills {
    let mut s: Skills = blank();
    for f in [
        &mut s.auralencalc,
        &mut s.aurarangecalc,
        &mut s.aurastatcalc1,
        &mut s.calc1,
        &mut s.calc2,
        &mut s.calc3,
        &mut s.calc4,
        &mut s.passivecalc1,
        &mut s.passivecalc2,
        &mut s.passivecalc3,
        &mut s.passivecalc4,
        &mut s.passivecalc5,
        &mut s.petmax,
        &mut s.skpoints,
        &mut s.tohitcalc,
        &mut s.dmgsympercalc,
        &mut s.edmgsympercalc,
        &mut s.elensympercalc,
    ] {
        *f = 0xFFFF_FFFF;
    }
    s.skilldesc = 0xFFFF;
    s.charclass = 0xFF;
    s.reqskill1 = 0xFFFF;
    s.reqskill2 = 0xFFFF;
    s.reqskill3 = 0xFFFF;
    s.itypea1 = 0xFFFF;
    s
}

/// A missiles record with no formulas.
pub fn missile_rec() -> Missiles {
    let mut m: Missiles = blank();
    m.dmgsympercalc = 0xFFFF_FFFF;
    m.edmgsympercalc = 0xFFFF_FFFF;
    m
}

/// Tables with `n` blank skills.
pub fn skill_tables(skills: Vec<Skills>) -> SkillTables {
    SkillTables {
        skills,
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: vec![missile_rec()],
        skills_code: Vec::new(),
        miss_code: Vec::new(),
        level_cap: super::LEVEL_CAP_114D,
        stat_count: 359,
    }
}

/// Combat tables: D2RUST's `charstats` factors and `difficultylevels`
/// values (the hit and damage constants), with the given monsters.
pub fn combat_tables(monsters: Vec<Monstats>) -> CombatTables {
    let tohit = [5i32, -15, -10, 20, 20, 5, 15];
    let block = [25u8, 20, 20, 30, 25, 20, 25];
    let charstats = (0..7)
        .map(|i| {
            let mut c: Charstats = blank();
            c.tohitfactor = tohit[i] as u32;
            c.blockfactor = block[i];
            c
        })
        .collect();
    let rows = [
        (0i32, 1u32, 1u32, 1u32, 1u32, 50u32),
        (-40, 2, 2, 2, 2, 35),
        (-100, 4, 4, 3, 3, 25),
    ];
    let difficultylevels = rows
        .iter()
        .map(|&(rp, fd, cd, ld, md, hb)| {
            let mut d: Difficultylevels = blank();
            d.resistpenalty = rp as u32;
            d.monsterfreezedivisor = fd;
            d.monstercolddivisor = cd;
            d.lifestealdivisor = ld;
            d.manastealdivisor = md;
            d.hireablebossdamagepercent = hb;
            d
        })
        .collect();
    let n = monsters.len();
    CombatTables {
        charstats,
        difficultylevels,
        monstats: monsters,
        monstats2: vec![blank::<Monstats2>(); n.max(1)],
        hitclass: [
            *b"none", *b"hth ", *b"1hss", *b"1hsl", *b"1ht ", *b"2hss", *b"2hsl", *b"2ht ",
            *b"bow ", *b"xbow", *b"club", *b"stf ", *b"ht2 ", *b"over",
        ]
        .to_vec(),
    }
}

/// A monstats record (monster class), killable, with velocity 1 and
/// Drain 100 on every difficulty.
pub fn monster_rec() -> Monstats {
    let mut m: Monstats = blank();
    m.killable = true;
    m.velocity = 1;
    m.drain = 100;
    m.drain_n = 100;
    m.drain_h = 100;
    m.montype = 0xFFFF;
    m
}
