//! A recording [`HirelingWorld`]: units in a map, stats per unit, every
//! seam call logged in order in [`Fake::log`], messages in [`Fake::sent`].

use std::collections::BTreeMap;

use crate::units::UnitId;
use crate::world::hirelings::{
    HirelingRow, HirelingRows, HirelingTables, HirelingWorld, RowSkill, UNIT_PLAYER,
};

#[derive(Debug, Clone, Default)]
pub struct FakeUnit {
    pub ty: u8,
    pub class: u32,
    pub guid: u32,
    pub mode: u32,
    pub flags: u32,
    pub flags2: u32,
    pub in_room: bool,
    /// Base values; totals add `bonus`.
    pub stats: BTreeMap<u16, i32>,
    pub bonus: BTreeMap<u16, i32>,
    pub state_stats: BTreeMap<(u16, u16), i32>,
    pub skills: BTreeMap<u32, i32>,
    pub owner: Option<(u32, u8)>,
    pub max_life: Option<i32>,
    /// The position the range follow reads.
    pub pos: (i32, i32),
}

#[derive(Debug, Default)]
pub struct Fake {
    pub units: BTreeMap<UnitId, FakeUnit>,
    pub expansion: bool,
    pub difficulty: u8,
    pub players: Vec<UnitId>,
    pub sent: Vec<(UnitId, Vec<u8>)>,
    /// The stat records queued per unit, in order.
    pub queued: BTreeMap<UnitId, Vec<(u16, u32)>>,
    pub log: Vec<String>,
    /// `skills` reqlevel per skill id; `skill_count`.
    pub reqlevel: BTreeMap<u32, i16>,
    pub skill_count: u32,
    /// `skill_pet_max` answer.
    pub pet_max: Option<i32>,
}

impl Fake {
    pub fn new(expansion: bool) -> Self {
        Self {
            expansion,
            skill_count: 400,
            ..Self::default()
        }
    }

    /// Adds a unit; players join the broadcast list.
    pub fn add(&mut self, id: u32, ty: u8, class: u32, guid: u32) -> UnitId {
        let u = UnitId(id);
        self.units.insert(
            u,
            FakeUnit {
                ty,
                class,
                guid,
                in_room: true,
                mode: 1,
                ..FakeUnit::default()
            },
        );
        if ty == UNIT_PLAYER {
            self.players.push(u);
        }
        u
    }

    pub fn unit(&self, u: UnitId) -> &FakeUnit {
        &self.units[&u]
    }
    pub fn unit_mut(&mut self, u: UnitId) -> &mut FakeUnit {
        self.units.get_mut(&u).expect("unit")
    }
    pub fn set(&mut self, u: UnitId, s: u16, v: i32) {
        self.unit_mut(u).stats.insert(s, v);
    }
    pub fn base(&self, u: UnitId, s: u16) -> i32 {
        self.unit(u).stats.get(&s).copied().unwrap_or(0)
    }
    /// The messages sent to `p`, in order.
    pub fn sent_to(&self, p: UnitId) -> Vec<Vec<u8>> {
        self.sent
            .iter()
            .filter(|(q, _)| *q == p)
            .map(|(_, b)| b.clone())
            .collect()
    }
}

impl HirelingWorld for Fake {
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn players(&self) -> Vec<UnitId> {
        self.players.clone()
    }
    fn send(&mut self, player: UnitId, bytes: &[u8]) {
        self.sent.push((player, bytes.to_vec()));
    }
    fn skill_pet_max(&self, _player: UnitId, pet_type: u8) -> Option<i32> {
        assert_eq!(pet_type, 7);
        self.pet_max
    }
    fn queue_stat(&mut self, unit: UnitId, stat: u16, value: u32) {
        self.queued.entry(unit).or_default().push((stat, value));
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.units.get(&unit).map_or(u32::MAX, |u| u.guid)
    }
    fn monster_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.units
            .iter()
            .find(|(_, u)| u.ty == 1 && u.guid == guid)
            .map(|(k, _)| *k)
    }
    fn player_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.units
            .iter()
            .find(|(_, u)| u.ty == 0 && u.guid == guid)
            .map(|(k, _)| *k)
    }
    fn unit_type(&self, unit: UnitId) -> u8 {
        self.units.get(&unit).map_or(0xFF, |u| u.ty)
    }
    fn class(&self, unit: UnitId) -> u32 {
        self.units.get(&unit).map_or(0, |u| u.class)
    }
    fn mode(&self, unit: UnitId) -> u32 {
        self.units.get(&unit).map_or(0, |u| u.mode)
    }
    fn set_mode(&mut self, unit: UnitId, mode: u8) {
        self.log.push(format!("mode {} {mode}", unit.0));
        self.unit_mut(unit).mode = u32::from(mode);
    }
    fn flags(&self, unit: UnitId) -> u32 {
        self.units.get(&unit).map_or(0, |u| u.flags)
    }
    fn set_flags(&mut self, unit: UnitId, flags: u32) {
        self.unit_mut(unit).flags = flags;
    }
    fn flags2(&self, unit: UnitId) -> u32 {
        self.units.get(&unit).map_or(0, |u| u.flags2)
    }
    fn set_flags2(&mut self, unit: UnitId, flags: u32) {
        self.unit_mut(unit).flags2 = flags;
    }
    fn in_room(&self, unit: UnitId) -> bool {
        self.units.get(&unit).is_some_and(|u| u.in_room)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.units.get(&unit).map_or((0, 0), |u| u.pos)
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.units.get(&unit).map_or(0, |u| {
            u.stats.get(&stat).copied().unwrap_or(0) + u.bonus.get(&stat).copied().unwrap_or(0)
        })
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.units
            .get(&unit)
            .and_then(|u| u.stats.get(&stat).copied())
            .unwrap_or(0)
    }
    fn set_base_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.log.push(format!("set {} {stat} {value}", unit.0));
        self.unit_mut(unit).stats.insert(stat, value);
    }
    fn max_life(&self, unit: UnitId) -> i32 {
        let u = &self.units[&unit];
        u.max_life
            .unwrap_or_else(|| u.stats.get(&7).copied().unwrap_or(0))
    }
    fn set_state_stat(&mut self, unit: UnitId, state: u16, stat: u16, value: i32) {
        self.log
            .push(format!("state_stat {} {state} {stat} {value}", unit.0));
        self.unit_mut(unit).state_stats.insert((state, stat), value);
    }
    fn remove_state(&mut self, unit: UnitId, state: u16) {
        self.log.push(format!("remove_state {} {state}", unit.0));
    }
    fn skill_count(&self) -> u32 {
        self.skill_count
    }
    fn skill_reqlevel(&self, skill: u32) -> Option<i16> {
        self.reqlevel.get(&skill).copied()
    }
    fn set_skill_level(&mut self, unit: UnitId, skill: u32, level: i32) {
        self.log.push(format!("skill {} {skill} {level}", unit.0));
        self.unit_mut(unit).skills.insert(skill, level);
    }
    fn set_owner(&mut self, merc: UnitId, guid: u32, unit_type: u8) {
        self.log
            .push(format!("owner {} {guid:#x} {unit_type}", merc.0));
        self.unit_mut(merc).owner = if guid == u32::MAX {
            None
        } else {
            Some((guid, unit_type))
        };
    }
    fn owner(&self, merc: UnitId) -> Option<(u32, u8)> {
        self.units.get(&merc).and_then(|u| u.owner)
    }
    fn join_team(&mut self, merc: UnitId, player: UnitId) {
        self.log.push(format!("team {} {}", merc.0, player.0));
    }
    fn hireling_ai(&mut self, merc: UnitId) {
        self.log.push(format!("ai {}", merc.0));
    }
    fn free_unit(&mut self, unit: UnitId) {
        self.log.push(format!("free {}", unit.0));
        self.units.remove(&unit);
    }
    fn queue_room_removal(&mut self, unit: UnitId) {
        self.log.push(format!("room_remove {}", unit.0));
    }
    fn death_event(&mut self, unit: UnitId) {
        self.log.push(format!("death_event {}", unit.0));
    }
    fn cancel_timers(&mut self, unit: UnitId, ty: u8, a: u32) {
        self.log.push(format!("cancel {} {ty} {a}", unit.0));
    }
    fn ensure_inventory(&mut self, unit: UnitId) {
        self.log.push(format!("inventory {}", unit.0));
    }
    fn dismiss(&mut self, unit: UnitId) {
        self.log.push(format!("dismiss {}", unit.0));
    }
    fn warp_to(&mut self, pet: UnitId, player: UnitId) {
        self.log.push(format!("warp {} {}", pet.0, player.0));
    }
    fn level_events(&mut self, player: UnitId, merc: UnitId) {
        self.log
            .push(format!("level_events {} {}", player.0, merc.0));
    }
    fn reapply_item_stats(&mut self, merc: UnitId) {
        self.log.push(format!("reapply {}", merc.0));
    }
}

/// A row with the given selectors and zeroed numbers.
pub fn row(version: u16, id: u32, act: u32, diff: u32, level: i32) -> HirelingRow {
    HirelingRow {
        version,
        id,
        class: 271,
        act,
        difficulty: diff,
        seller: 150,
        level,
        ..HirelingRow::default()
    }
}

/// Live base-game expansion Act 1 Normal Ice (Id 1) brackets 3 / 36 / 67
/// (the values the reference vectors imply).
/// Only the columns the Act 1 Ice vectors need are filled.
pub fn act1_ice_rows() -> Vec<HirelingRow> {
    let mk = |level: i32| HirelingRow {
        version: 100,
        id: 1,
        class: 271,
        act: 1,
        difficulty: 1,
        seller: 150,
        level,
        ..HirelingRow::default()
    };
    vec![mk(3), mk(36), mk(67)]
}

/// Tables with the base-game `pettype` row 7 flags (warp 1, range 0,
/// basemax 1) and MaxLvl 99.
pub fn tables(rows: Vec<HirelingRow>) -> HirelingTables {
    HirelingTables {
        rows: HirelingRows::new(rows),
        exp_ratios: Default::default(),
        max_level: 99,
        pet_flags: HirelingTables::WARP,
        pet_basemax: 1,
    }
}

/// A row skill.
pub fn skill(skill: i32, mode: i8, level: i8, lvl_per_lvl: i8) -> RowSkill {
    RowSkill {
        skill,
        mode,
        level,
        lvl_per_lvl,
    }
}
