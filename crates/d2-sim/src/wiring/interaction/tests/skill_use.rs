//! Skill use → missiles → combat: a player's skill do creates a real
//! missile through the missile code, the unit dispatch flies it through
//! a real room grid and it hits a real monster; mana is charged on the
//! real stat lists; the cooldown list and its expiry timer are real.
//!
//! The fixture is the action wiring's (a synthetic DRLG level of two 8×8
//! preset rooms, streamed; an arrow-like missile), rebuilt here because
//! that module's test fixture is private to it.

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::tables::{Levels, Missiles as MissileRow};

use crate::drlg::collision::bits;
use crate::drlg::room::LinkAt;
use crate::drlg::tiles::{cell, FIXED_LIBRARY};
use crate::drlg::{
    CellGrid, Drlg, DrlgData, DrlgError, DrlgRoomId, Dungeon, GridPass, LevelDef, LevelIdx,
    LevelTypes, RoomGrids, RoomKind as DrlgRoomKind, TileInfo, TileRect, TileSource,
};
use crate::game::Game;
use crate::missiles::unit_flag;
use crate::rng::Seed;
use crate::skills::fake::{blank, combat_tables, monster_rec, skill_rec, skill_tables};
use crate::skills::use_::{
    do_skill, set_delay, ModeTarget, ServerMsg, UseState, FLAG_MISSILE_FIRED,
};
use crate::skills::{SkillEntry, SkillTables};
use crate::stats::stat as sst;
use crate::stats::StatData;
use crate::tick::events::event;
use crate::units::hooks::Sim;
use crate::units::hooks::{MonsterInfo, UnitData};
use crate::units::lifecycle::AllocRequest;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::action::{
    ActionHooks, ActionSim, ActionTables, DrlgWorld, Pending, SkillEvent, WiringError,
};
use crate::wiring::interaction::UseRest;

const INIT: u32 = 644_409_375;
const LEVEL: u32 = 2;
const TOHIT: u16 = 19;
const LEVEL_STAT: u16 = 12;
const ARMORCLASS: u16 = 31;
const MINDAMAGE: u16 = 21;
const MAXDAMAGE: u16 = 22;
/// The test skill: missile 0, mana 2 points (2 << 8).
const SKILL: i32 = 0;

// ---- the open seams: path, hostility, the skill list -----------------------

/// A straight-line path (one sub-tile per step toward the target),
/// positions, hostility (everyone but oneself), the unit's skill list and
/// used skill, and the skill missile record fill (aim at `aim_at`).
#[derive(Default)]
pub(super) struct Open {
    pos: BTreeMap<UnitId, (i32, i32)>,
    dir: BTreeMap<UnitId, i32>,
    /// The path target point set at creation.
    target_point: BTreeMap<UnitId, (i32, i32)>,
    crossed: BTreeMap<UnitId, Vec<(i32, i32)>>,
    velocity: BTreeMap<UnitId, i32>,
    pub(super) skills: BTreeMap<UnitId, Vec<SkillEntry>>,
    pub(super) used: BTreeMap<UnitId, SkillEntry>,
    /// Right skills (`UseRest::right_skill`).
    pub(super) right: BTreeMap<UnitId, SkillEntry>,
    pub(super) aim_at: (i32, i32),
    pub(super) log: Vec<String>,
    /// Bytes sent to a player's client (`Pending::send`).
    pub(super) msgs: Vec<(UnitId, Vec<u8>)>,
    /// Path targets (`UseRest::target`).
    pub(super) targets: BTreeMap<UnitId, UnitId>,
    /// Items by (unit, body location); the current weapon; stackable
    /// items (the skill bodies' item seams).
    pub(super) items: BTreeMap<(UnitId, u8), UnitId>,
    pub(super) weapon: BTreeMap<UnitId, UnitId>,
    pub(super) stackable: Vec<UnitId>,
    item_types: BTreeMap<UnitId, Vec<i32>>,
    max_stacks: BTreeMap<UnitId, i32>,
    /// Alignment per unit (`Pending::alignment`; absent: evil, 0).
    pub(super) align: BTreeMap<UnitId, u8>,
}

impl Pending for Open {
    fn alignment(&self, unit: UnitId) -> u8 {
        self.align.get(&unit).copied().unwrap_or(0)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or((0, 0))
    }
    fn place(&mut self, unit: UnitId, x: i32, y: i32) {
        self.pos.insert(unit, (x, y));
    }
    fn size(&self, _: UnitId) -> i32 {
        1
    }
    fn has_path(&self, _: UnitId) -> bool {
        true
    }
    fn set_velocity(&mut self, unit: UnitId, v: i32) {
        self.velocity.insert(unit, v);
    }
    fn velocity(&self, unit: UnitId) -> i32 {
        self.velocity.get(&unit).copied().unwrap_or(0)
    }
    fn set_target_point(&mut self, unit: UnitId, x: i32, y: i32) {
        let (ux, _) = self.position(unit);
        self.dir.insert(unit, (x - ux).signum());
        self.target_point.insert(unit, (x, y));
    }
    /// The lob's life is short enough to land on the test target.
    fn target_distance(&self, _: UnitId) -> i32 {
        3
    }
    fn step(&mut self, _: &mut Game, unit: UnitId) -> bool {
        let (x, y) = self.position(unit);
        // A lob that reached its target point waits there for the
        // activation frame instead of flying past it.
        if let Some(&(tx, _)) = self.target_point.get(&unit) {
            if x == tx {
                self.crossed.insert(unit, vec![(x, y)]);
                return true;
            }
        }
        let d = self.dir.get(&unit).copied().unwrap_or(1);
        let p = (x + if d == 0 { 1 } else { d }, y);
        self.pos.insert(unit, p);
        self.crossed.insert(unit, vec![p]);
        true
    }
    fn crossed_subtiles(&self, unit: UnitId) -> Vec<(i32, i32)> {
        self.crossed.get(&unit).cloned().unwrap_or_default()
    }
    fn may_attack(&self, a: UnitId, d: UnitId) -> bool {
        a != d
    }
    fn class_has_mode(&self, _: i32, _: u8) -> bool {
        true
    }
    fn skill_list(&self, unit: UnitId) -> Vec<SkillEntry> {
        self.skills.get(&unit).cloned().unwrap_or_default()
    }
    fn used_skill(&self, unit: UnitId) -> Option<SkillEntry> {
        self.used.get(&unit).copied()
    }
    fn reaction(&mut self, a: UnitId, d: UnitId, r: &mut crate::combat::DamageRecord) {
        self.log
            .push(format!("reaction {} {} {:#x}", a.0, d.0, r.result));
    }
    fn skill_event(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, ev: SkillEvent) {
        crate::wiring::interaction::skill_events::route(h, sim, ev);
    }
    fn assign_right_aura(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId) {
        crate::wiring::interaction::skill_events::assign_right_aura(h, sim, unit);
    }
    fn monster_right_aura(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        unit: UnitId,
        skill: i32,
        level: i32,
    ) {
        crate::wiring::interaction::skill_events::monster_right_aura(h, sim, unit, skill, level);
    }
    fn monster_passive_refresh(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        unit: UnitId,
        skill: i32,
        level: i32,
    ) {
        crate::wiring::interaction::skill_events::monster_passive_refresh(
            h, sim, unit, skill, level,
        );
    }
    fn monster_attack_skill(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId) {
        crate::wiring::interaction::skill_events::monster_attack_skill(h, sim, unit);
    }
    fn monster_attack_strike(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId, m: bool) {
        crate::wiring::interaction::skill_events::monster_attack_strike(h, sim, unit, m);
    }
    fn item_at(&self, unit: UnitId, loc: u8) -> Option<UnitId> {
        self.items.get(&(unit, loc)).copied()
    }
    fn current_weapon(&self, unit: UnitId) -> Option<UnitId> {
        self.weapon.get(&unit).copied()
    }
    fn item_stackable(&self, item: UnitId) -> bool {
        self.stackable.contains(&item)
    }
    fn has_inventory(&self, unit: UnitId) -> bool {
        self.items.keys().any(|&(owner, _)| owner == unit)
    }
    fn weapon_in_use(&self, unit: UnitId) -> Option<UnitId> {
        self.current_weapon(unit)
    }
    fn item_usable(&self, item: UnitId) -> bool {
        self.stackable.contains(&item)
    }
    fn item_is(&self, item: UnitId, ty: i32) -> bool {
        self.item_types
            .get(&item)
            .is_some_and(|types| types.contains(&ty))
    }
    fn item_flag_throw(&self, item: UnitId) -> bool {
        self.stackable.contains(&item)
    }
    fn item_max_stack(&self, item: UnitId) -> i32 {
        self.max_stacks.get(&item).copied().unwrap_or(0)
    }
    fn set_ai_state(&mut self, unit: UnitId, k: i32) {
        self.log.push(format!("ai {} {k}", unit.0));
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.msgs.push((player, msg.to_vec()));
    }
}

impl UseRest for Open {
    fn send(&mut self, u: UnitId, msg: ServerMsg) {
        self.log.push(format!("send {} {msg:?}", u.0));
    }
    fn has_player_data(&self, _: UnitId) -> bool {
        true
    }
    fn last_point_frame(&self, _: UnitId) -> i32 {
        0
    }
    fn set_last_point_frame(&mut self, _: UnitId, _: i32) {}
    fn cursor_item(&self, _: UnitId) -> bool {
        false
    }
    fn in_own_inventory(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn within_reach(&self, _: UnitId, _: UnitId) -> bool {
        true
    }
    fn owner(&self, _: UnitId) -> Option<UnitId> {
        None
    }
    fn is_pet(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn is_ally(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn left_skill(&self, _: UnitId) -> Option<SkillEntry> {
        None
    }
    fn right_skill(&self, u: UnitId) -> Option<SkillEntry> {
        self.right.get(&u).copied()
    }
    fn set_left_skill(&mut self, _: UnitId, _: SkillEntry) {}
    fn set_right_skill(&mut self, _: UnitId, _: SkillEntry) {}
    fn find_entry(&self, u: UnitId, skill: i32) -> Option<SkillEntry> {
        self.skill_list(u).into_iter().find(|e| e.skill == skill)
    }
    fn find_entry_owned(&self, _: UnitId, _: i32, _: i32) -> Option<SkillEntry> {
        None
    }
    fn owns_skill(&self, u: UnitId, skill: i32) -> bool {
        self.find_entry(u, skill).is_some()
    }
    fn set_used_skill(&mut self, u: UnitId, e: Option<SkillEntry>) {
        match e {
            Some(e) => self.used.insert(u, e),
            None => self.used.remove(&u),
        };
    }
    fn used_skill_flags(&self, _: UnitId) -> u32 {
        0
    }
    fn set_used_skill_flags(&mut self, _: UnitId, _: u32) {}
    fn entry_mode(&self, _: UnitId, _: &SkillEntry) -> u32 {
        0
    }
    fn attack_param4(&self, _: UnitId) -> i32 {
        0
    }
    fn set_attack_param4(&mut self, _: UnitId, _: i32) {}
    fn use_state(&mut self, _: UnitId, _: &SkillEntry) -> UseState {
        UseState::Usable
    }
    fn shapeshifted(&self, _: UnitId) -> bool {
        false
    }
    fn consume_charges(&mut self, _: UnitId, _: &SkillEntry) -> bool {
        true
    }
    fn pay_life(&mut self, _: UnitId, _: i32) -> bool {
        true
    }
    fn can_dual_wield(&self, _: UnitId) -> bool {
        false
    }
    fn equippable(&self, _: UnitId) -> bool {
        false
    }
    fn bow_equipped(&self, _: UnitId) -> bool {
        false
    }
    fn state_mask(&self, _: UnitId, _: u32) -> bool {
        false
    }
    fn start_mode(&mut self, _: &mut Game, _: UnitId, _: u32, _: ModeTarget<UnitId>) {}
    fn run_to(&mut self, _: UnitId, _: UnitId, _: SkillEntry) {}
    fn target(&self, u: UnitId) -> Option<UnitId> {
        self.targets.get(&u).copied()
    }
    fn clear_target(&mut self, _: UnitId) {}
    fn event_arg(&self, _: UnitId) -> i32 {
        0
    }
    fn set_event_arg(&mut self, _: UnitId, _: i32) {}
    fn step_path(&mut self, _: UnitId) -> i32 {
        0
    }
    fn target_position(&self, _: UnitId) -> Option<(i32, i32)> {
        Some(self.aim_at)
    }
    fn line_clear(&self, _: UnitId, _: (i32, i32), _: u32) -> bool {
        true
    }
    fn set_aura_state(&mut self, _: UnitId, _: u16, _: i32, _: i32) {}
    fn srvst(&mut self, _: u16, _: UnitId, _: i32, _: i32) -> i32 {
        1
    }
    fn srvdo(&mut self, i: u16, _: UnitId, _: i32, _: i32, _: bool, _: bool, _: bool) -> i32 {
        self.log.push(format!("srvdo {i}"));
        0
    }
}

// ---- DRLG fixture (the action wiring's) --------------------------------------

struct Types {
    rooms: Vec<TileRect>,
}

impl LevelTypes for Types {
    fn generate(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        level: LevelIdx,
    ) -> Result<(), DrlgError> {
        for rect in self.rooms.clone() {
            let r = drlg.alloc_room(level, DrlgRoomKind::Preset, rect);
            drlg.room_mut(r).dt1_mask = 1;
            drlg.link_room(r, LinkAt::Tail);
        }
        Ok(())
    }

    fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        _: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        let r = drlg.room(room).rect;
        let (w, h) = (r.w as usize + 1, r.h as usize + 1);
        let mut g = CellGrid::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                g.set(x, y, cell::FLOOR | if edge { cell::LINKED } else { 0 });
            }
        }
        Ok(RoomGrids {
            passes: vec![GridPass {
                cells: g,
                orientation: None,
                fill_blanks: false,
            }],
            ..RoomGrids::default()
        })
    }
}

fn tile(o: u32, main: u32, sub: u32, rarity: u32) -> TileInfo {
    TileInfo {
        orientation: o,
        main,
        sub,
        rarity,
        material: 0,
        subtile_flags: [0; 25],
        roof_height: 0,
        height: 0,
        light_direction: 0,
    }
}

struct Tiles(BTreeMap<Vec<u8>, Vec<TileInfo>>);

impl TileSource for Tiles {
    fn dt1(&self, path: &[u8]) -> Option<&[TileInfo]> {
        self.0.get(path).map(Vec::as_slice)
    }
}

fn tiles() -> Tiles {
    let mut t = BTreeMap::new();
    let mut floor: Vec<TileInfo> = (0..4).map(|_| tile(0, 0, 0, 1)).collect();
    for o in [1, 2, 3, 4, 13] {
        floor.push(tile(o, 1, 0, 1));
    }
    t.insert(b"floor.dt1".to_vec(), floor);
    let blank_tile = |sub| {
        let mut x = tile(0, 30, sub, 0);
        x.subtile_flags = [0x20; 25];
        x
    };
    t.insert(
        FIXED_LIBRARY[0].to_vec(),
        vec![blank_tile(0), blank_tile(1)],
    );
    t.insert(FIXED_LIBRARY[1].to_vec(), vec![]);
    t.insert(FIXED_LIBRARY[2].to_vec(), vec![tile(10, 0, 0, 0)]);
    Tiles(t)
}

fn drlg_data() -> DrlgData {
    let mut d = DrlgData {
        levels: vec![LevelDef::default(); 150],
        ..DrlgData::default()
    };
    for l in &mut d.levels {
        l.warp = [-1; 8];
    }
    let mut files = vec![Vec::new(); 32];
    files[0] = b"floor.dt1".to_vec();
    d.lvltypes = vec![vec![Vec::new(); 32], files];
    d.levels[LEVEL as usize].drlg_type = 2;
    d.levels[LEVEL as usize].level_type = 1;
    d
}

/// Missile 0: an arrow (default flight, collide type 3, kill on
/// collision, to-hit).
fn arrow() -> MissileRow {
    let mut r: MissileRow = blank();
    r.psrvdofunc = 1;
    r.vel = 1;
    r.maxvel = 1;
    r.range = 50;
    r.collidetype = 3;
    r.collidekill = 1;
    r.lastcollide = true;
    r.tohit = 1;
    r.size = 1;
    r
}

/// Skill 0: `srvmissile` 0, mana 2 (shift 8), no start / do function.
pub(super) fn skills() -> SkillTables {
    let mut s = skill_rec();
    s.srvmissile = 0;
    s.mana = 2;
    s.manashift = 8;
    let mut t = skill_tables(vec![s]);
    t.missiles = vec![arrow()];
    t
}

fn monster_class() -> d2_data::tables::Monstats {
    let mut m = monster_rec();
    m.skill1 = 0xFFFF;
    m.skill2 = 0xFFFF;
    m.skill3 = 0xFFFF;
    m
}

pub(super) struct Fx {
    pub(super) game: Game,
    pub(super) sim: ActionSim<Open>,
    room: RoomId,
    skills: SkillTables,
}

impl Fx {
    fn new() -> Self {
        Self::with(super::stat_data(), skills())
    }

    /// The fixture on other stat data and skill tables (the skill's
    /// missile stays the arrow).
    pub(super) fn with(stat_data: Arc<StatData>, mut skills: SkillTables) -> Self {
        skills.missiles = vec![arrow()];
        let data = drlg_data();
        let mut types = Types {
            rooms: vec![TileRect::new(0, 0, 8, 8), TileRect::new(8, 0, 8, 8)],
        };
        let drlg = Drlg::create(0, INIT, 0, 0, false, &data, &mut types).expect("drlg");
        let mut dungeon = Dungeon::default();
        dungeon.acts[0] = Some(drlg);
        let world = DrlgWorld {
            dungeon,
            data: Arc::new(data),
            tiles: Box::new(tiles()),
            types: Box::new(types),
        };
        let tables = ActionTables {
            missiles: vec![arrow()],
            skills: skills.clone(),
            combat: combat_tables(vec![monster_class()]),
            levels: vec![blank::<Levels>(); 150],
            skill_modes: vec![[0; 8]],
            overlay_count: 0,
            monequip: Vec::new(),
            arena: Vec::new(),
        };
        let hooks = ActionHooks::new(
            Arc::new(tables),
            world,
            Seed::init_low(1234),
            Open::default(),
        );
        let unit_data = UnitData {
            monsters: vec![MonsterInfo {
                enabled: true,
                aidel: [15, 15, 15],
                moves: 1 << 4,
                mode_chart: false,
            }],
            ..UnitData::default()
        };
        let mut sim = ActionSim::new(stat_data, unit_data, hooks);
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let rooms = sim
            .hooks()
            .drlg
            .with_act(0, &mut game.lists, |d, svc| {
                let l = d.get_or_alloc_level(svc.data, svc.types, LEVEL)?;
                d.generate_level(svc.data, svc.types, l)?;
                let mut out = Vec::new();
                for r in d.level_rooms(l) {
                    out.push(d.stream_room(svc, r)?.expect("active"));
                }
                Ok::<_, DrlgError>(out)
            })
            .unwrap()
            .unwrap();
        Self {
            game,
            sim,
            room: rooms[0],
            skills,
        }
    }

    pub(super) fn spawn(&mut self, ty: UnitType, x: i32, y: i32) -> UnitId {
        let req = AllocRequest {
            ty,
            class: 0,
            room: Some(self.room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: ty == UnitType::Player,
        };
        let u = self
            .sim
            .with(&mut self.game, |g, v| v.allocate(g, &req, x, y))
            .expect("allocated");
        self.sim.sys.units.get_mut(u).unwrap().mode = 1;
        u
    }

    pub(super) fn set(&mut self, u: UnitId, values: &[(u16, i32)]) {
        self.sim.with(&mut self.game, |_, v| {
            for &(s, x) in values {
                v.set_base(u, s, x);
            }
        });
    }

    fn stat(&mut self, u: UnitId, s: u16) -> i32 {
        self.sim.with(&mut self.game, |_, v| v.stat(u, s))
    }

    /// Marks a unit's collision bit at its position (movement's job).
    fn mark(&mut self, u: UnitId, bit: u16) {
        let (x, y) = self.sim.hooks().x.position(u);
        let game = &self.game;
        *self
            .sim
            .sys
            .hooks
            .drlg
            .collision_mut(game, self.room, x, y)
            .expect("in a grid") |= bit;
    }

    pub(super) fn frame(&mut self) {
        self.game.frame += 1;
        crate::tick::run_timer_events(&mut self.game, &mut self.sim);
    }

    pub(super) fn assert_clean(&self) {
        assert_eq!(self.sim.sys.hooks.errors, Vec::<WiringError>::new());
        assert!(self.sim.sys.errors.is_empty(), "{:?}", self.sim.sys.errors);
    }
}

/// A seed whose next step gives `lo′ = x`.
fn seed_giving(x: u32) -> Seed {
    Seed::new(0, x)
}

#[test]
fn skill_do_fires_a_real_missile_that_hits_a_real_monster() {
    let mut fx = Fx::new();
    let player = fx.spawn(UnitType::Player, 10, 10);
    let monster = fx.spawn(UnitType::Monster, 13, 10);
    // Player: AR 300, level 10, mana 20 points; the monster: defense 100,
    // level 8, life 100 (to-hit chance 83).
    fx.set(
        player,
        &[
            (TOHIT, 300),
            (LEVEL_STAT, 10),
            (sst::MAXMANA, 20 << 8),
            (8, 20 << 8),
        ],
    );
    fx.set(
        monster,
        &[
            (ARMORCLASS, 100),
            (LEVEL_STAT, 8),
            (sst::MAXHP, 25600),
            (sst::HITPOINTS, 25600),
        ],
    );
    fx.sim.sys.units.get_mut(monster).unwrap().flags |=
        unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED;
    fx.mark(monster, bits::MONSTER);
    fx.sim.sys.units.get_mut(player).unwrap().seed = seed_giving(10);
    let entry = SkillEntry {
        skill: SKILL,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    {
        let x = &mut fx.sim.hooks().x;
        x.skills.insert(player, vec![entry]);
        x.used.insert(player, entry);
        x.aim_at = (13, 10);
    }
    // The skill do at level 1.
    let t = fx.skills.clone();
    let game = &mut fx.game;
    let r = fx
        .sim
        .skill_use(game, |w| do_skill(w, &t, player, SKILL, 1));
    assert_eq!(r, 1, "srvmissile → result 1 (§5.4 step 7)");
    // Flag 0x40 on the caster; mana charged (2 points).
    assert_ne!(
        fx.sim.sys.units.get(player).unwrap().flags & FLAG_MISSILE_FIRED,
        0
    );
    assert_eq!(fx.stat(player, 8), 18 << 8);
    // The real missile: allocated, in the store, every-tick event, at the
    // caster's position, with the skill and level stored.
    let missiles = fx.game.lists.units_of_type(UnitType::Missile);
    assert_eq!(missiles.len(), 1);
    let m = missiles[0];
    let data = fx.sim.hooks().missile_store().get(m).unwrap().clone();
    assert_eq!((i32::from(data.skill), data.level), (SKILL, 1));
    assert_eq!(fx.sim.hooks().x.position(m), (10, 10));
    // The skill do doesn't set the missile's damage; the test does.
    fx.set(m, &[(MINDAMAGE, 2560), (MAXDAMAGE, 2560)]);
    // Runs 1–3: two empty sub-tiles, then the monster's; to-hit 10 < 83
    // on the owner's seed; 2560 off the monster's life; removed.
    fx.frame();
    fx.frame();
    assert_eq!(fx.stat(monster, sst::HITPOINTS), 25600);
    fx.frame();
    assert_eq!(fx.stat(monster, sst::HITPOINTS), 25600 - 2560);
    assert!(fx.game.lists.unit(m).is_none());
    let mut want = seed_giving(10);
    want.step();
    assert_eq!(fx.sim.sys.units.get(player).unwrap().seed, want);
    assert!(fx
        .sim
        .hooks()
        .x
        .log
        .contains(&format!("reaction {} {} 0x1", player.0, monster.0)));
    fx.assert_clean();
}

#[test]
fn equipped_throw_spends_one_and_its_real_missile_damages_a_monster() {
    for (ty, gas) in [(45, false), (38, false), (38, true)] {
        let mut fx = Fx::new();
        let throw = 2;
        let mut row = fx.skills.skills[0].clone();
        row.srvdofunc = 3;
        row.srvmissile = 0xFFFF;
        fx.skills.skills.resize(3, row.clone());
        fx.skills.skills[throw as usize] = row;
        let mut missile = arrow();
        missile.mindamage = 10;
        missile.maxdamage = 10;
        missile.hitshift = 8;
        missile.dmgsympercalc = u32::MAX;
        let mut rows = vec![missile];
        if gas {
            rows[0].psrvhitfunc = 2;
            rows[0].hitsubmissile1 = 1;
            rows[0].shitpar2 = 2;
            let mut cloud = arrow();
            cloud.psrvdofunc = 3;
            // The ring gives the cloud the row's param1 as its velocity.
            cloud.param1 = 1;
            cloud.vel = 0;
            cloud.maxvel = 0;
            // A cloud always lands: no to-hit roll.
            cloud.tohit = 0;
            cloud.etype = 5;
            cloud.emin = 10;
            cloud.emax = 10;
            cloud.elen = 50;
            cloud.edmgsympercalc = u32::MAX;
            rows.push(cloud);
        }
        fx.skills.missiles = rows.clone();
        let tables = Arc::make_mut(&mut fx.sim.sys.hooks.tables);
        tables.missiles = rows;
        tables.skills = fx.skills.clone();
        let player = fx.spawn(UnitType::Player, 10, 10);
        let monster = fx.spawn(UnitType::Monster, 13, 10);
        let item = fx.spawn(UnitType::Item, 10, 10);
        fx.set(player, &[(TOHIT, 300), (LEVEL_STAT, 10), (8, 20 << 8)]);
        fx.set(item, &[(70, 2)]);
        fx.set(
            monster,
            &[
                (ARMORCLASS, 100),
                (LEVEL_STAT, 8),
                (sst::MAXHP, 25600),
                (sst::HITPOINTS, 25600),
            ],
        );
        fx.sim.sys.units.get_mut(monster).unwrap().flags |=
            unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED;
        fx.mark(monster, bits::MONSTER);
        fx.sim.sys.units.get_mut(player).unwrap().seed = seed_giving(10);
        let entry = SkillEntry {
            skill: throw,
            base: 1,
            owner_guid: -1,
            ..SkillEntry::default()
        };
        let x = &mut fx.sim.hooks().x;
        x.skills.insert(player, vec![entry]);
        x.used.insert(player, entry);
        x.items.insert((player, 4), item);
        x.stackable.push(item);
        x.item_types.insert(item, vec![ty]);
        x.max_stacks.insert(item, 25);
        if ty == 45 {
            x.weapon.insert(player, item);
        }
        x.aim_at = (13, 10);
        let t = fx.skills.clone();
        assert_eq!(
            fx.sim
                .skill_use(&mut fx.game, |w| crate::skills::use_::bodies::starts::ammo(
                    w, player
                )),
            1
        );
        assert_eq!(
            fx.sim
                .skill_use(&mut fx.game, |w| do_skill(w, &t, player, throw, 1)),
            1
        );
        assert_eq!(fx.stat(item, 70), 1, "item type {ty}");
        let missiles = fx.game.lists.units_of_type(UnitType::Missile);
        assert_eq!(missiles.len(), 1);
        assert_eq!(fx.stat(missiles[0], MINDAMAGE), 2560);
        // A potion is lobbed and lands later than the straight throw.
        let flight = if ty == 38 { 30 } else { 3 };
        for _ in 0..flight {
            fx.frame();
        }
        if gas {
            assert!(
                fx.game
                    .lists
                    .units_of_type(UnitType::Missile)
                    .iter()
                    .any(|&m| fx.sim.sys.units.get(m).unwrap().class == 1),
                "gas impact creates poison clouds"
            );
        }
        for _ in 0..10 {
            fx.frame();
        }
        assert!(fx.stat(monster, sst::HITPOINTS) < 25600, "item type {ty}");
        if gas {
            assert!(
                fx.sim.sys.stats.has_state(monster, 2),
                "the cloud poisons the monster"
            );
        }
        assert_eq!(fx.stat(item, 70), 1);
        fx.assert_clean();
    }
}

#[test]
fn cooldown_list_and_its_expiry_timer_are_real() {
    let mut fx = Fx::new();
    let player = fx.spawn(UnitType::Player, 10, 10);
    fx.game.frame = 100;
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| set_delay(w, player, 25));
    // A state-121 list expiring at 125, the state on, a type-12 timer.
    let (list, on) = fx.sim.with(&mut fx.game, |_, v| {
        (v.state_list(player, 121), v.stats.has_state(player, 121))
    });
    let list = list.expect("skill delay list");
    assert!(on);
    assert_eq!(fx.sim.sys.stats.expire(list), 125);
    let t = &fx.game.timers;
    let expiry: Vec<_> = t
        .unit_timers(player)
        .into_iter()
        .filter(|&id| t.event(id).is_some_and(|e| e.0 == event::REMOVE_STATE))
        .filter_map(|id| t.expire(id))
        .collect();
    assert_eq!(expiry, [125]);
    // A second delay moves the expiry and schedules another timer.
    fx.game.frame = 110;
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| set_delay(w, player, 25));
    assert_eq!(fx.sim.sys.stats.expire(list), 135);
    // Frame 125 runs the type-12 expiry: the list
    // stays until its own expire frame.
    for _ in 110..136 {
        fx.frame();
    }
    let gone = fx
        .sim
        .with(&mut fx.game, |_, v| v.state_list(player, 121).is_none());
    assert!(gone);
    fx.assert_clean();
}

#[test]
fn item_event_layer_split_is_the_1_14d_stuff() {
    // `stuff` 6, mask (1 << 6) − 1: a registration layer (skill << 6) +
    // level splits back into skill and level.
    let (shift, mask) = Open::default().event_layer_split();
    assert_eq!((shift, mask), (6, 0x3F));
    let layer = (54u32 << shift) + 13;
    assert_eq!((layer >> shift, layer & mask), (54, 13));
}

#[test]
fn use_line_clear_sees_a_wall_on_the_rooms_once_paths_are_on() {
    use crate::skills::use_::UseWorld;
    // Without the path provider the seam answers (the fixture's: clear).
    let mut off = Fx::new();
    let p = off.spawn(UnitType::Player, 10, 10);
    assert!(off
        .sim
        .skill_use(&mut off.game, |w| w.line_clear(p, (16, 10), 0x805)));
    // The server turns the provider on before any unit is allocated.
    let mut fx = Fx::new();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let player = fx.spawn(UnitType::Player, 10, 10);
    let clear = fx
        .sim
        .skill_use(&mut fx.game, |w| w.line_clear(player, (16, 10), 0x805));
    assert!(clear, "no wall yet");
    let room = fx.room;
    *fx.sim
        .sys
        .hooks
        .drlg
        .collision_mut(&fx.game, room, 13, 10)
        .expect("in a grid") |= bits::WALL;
    let blocked = fx
        .sim
        .skill_use(&mut fx.game, |w| w.line_clear(player, (16, 10), 0x805));
    assert!(!blocked, "the wall between the two");
    fx.assert_clean();
}

#[test]
fn use_point_collides_reads_the_rooms_once_paths_are_on() {
    use crate::skills::use_::bodies::BodyWorld;
    // The collision check of Armageddon's tries: without the provider the
    // seam's default (collides); with it the cell's value under the mask.
    let mut fx = Fx::new();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let _player = fx.spawn(UnitType::Player, 10, 10);
    let room = fx.room;
    let at = |fx: &mut Fx, m| {
        fx.sim.skill_use(&mut fx.game, |w| {
            BodyWorld::point_collides(w, room, (13, 10), m)
        })
    };
    assert!(!at(&mut fx, 1), "an open cell");
    *fx.sim
        .sys
        .hooks
        .drlg
        .collision_mut(&fx.game, room, 13, 10)
        .expect("in a grid") |= bits::WALL;
    assert!(at(&mut fx, 1), "a wall under mask 1");
    assert!(!at(&mut fx, 0), "nothing under mask 0");
    fx.assert_clean();
}

#[test]
fn missile_data_words_reach_the_missile_store() {
    use crate::skills::use_::bodies::{BodyEffect, BodyWorld, MissileRequest};
    // Volcano writes its seed word to the new missile's first data word
    // (`MissileData28`); server-do 28 re-seeds from it. The second word
    // (`MissileData2C`) likewise.
    let mut fx = Fx::new();
    let player = fx.spawn(UnitType::Player, 10, 10);
    let m = fx.sim.skill_use(&mut fx.game, |w| {
        let m = w.spawn_missile(MissileRequest {
            flags: 1,
            x: 12,
            y: 10,
            ..MissileRequest::new(player, 0)
        })?;
        w.effect(BodyEffect::MissileData28 { missile: m, v: 174 });
        w.effect(BodyEffect::MissileData2C { missile: m, v: -3 });
        Some(m)
    });
    let m = m.expect("created");
    let data = fx.sim.hooks().missile_store().get(m).unwrap().clone();
    assert_eq!(data.target, (174, -3));
    fx.assert_clean();
}

#[test]
fn inferno_frames_reach_the_missile_store() {
    use crate::skills::use_::bodies::{BodyWorld, MissileRequest};
    // The Inferno frames helper sets the new missile's total frames and
    // frames left, and DeathMaul reads the total back: both on the real
    // store, so DiabLight's bolts live `Param2 + L − 1` frames, not
    // `Range` (gen-ai-diablo: 32, not 30).
    let mut fx = Fx::new();
    let player = fx.spawn(UnitType::Player, 10, 10);
    let got = fx.sim.skill_use(&mut fx.game, |w| {
        let m = w.spawn_missile(MissileRequest {
            flags: 1,
            x: 12,
            y: 10,
            ..MissileRequest::new(player, 0)
        })?;
        w.set_missile_frames(m, 32, 31);
        let a = w.missile_frames(m);
        w.set_missile_frames(m, 0x9000, -0x9000);
        Some((m, a, w.missile_frames(m)))
    });
    let (m, a, b) = got.expect("created");
    assert_eq!((a, b), (32, 0x7FFF));
    let data = fx.sim.hooks().missile_store().get(m).unwrap().clone();
    assert_eq!((data.total, data.current), (0x7FFF, -0x8000));
    fx.assert_clean();
}

#[test]
fn use_allied_resolves_a_monster_to_its_minion_owner() {
    use crate::monsters::ai::{AiControl, AiStore, UnitRef};
    use crate::skills::use_::bodies::BodyWorld;
    // Filter 0x10000: a monster stands for its minion owner; the same
    // unit after that → allies. An Oak
    // Sage's aura (filter 0x10103) reaches its druid this way.
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 10, 10);
    let pet = fx.spawn(UnitType::Monster, 11, 10);
    let wild = fx.spawn(UnitType::Monster, 12, 10);
    let guid = fx.sim.sys.units.get(p).unwrap().guid;
    let mut ai = AiStore::new();
    ai.entry(pet).control = Some(AiControl {
        minion_owner: Some(UnitRef {
            ty: UnitType::Player,
            guid,
        }),
        ..AiControl::default()
    });
    ai.entry(wild).control = Some(AiControl::default());
    fx.sim.sys.hooks.ai = Some(ai);
    let wild2 = fx.spawn(UnitType::Monster, 13, 10);
    // The player is good (2), the wild monsters evil (0).
    fx.sim.hooks().x.align.insert(p, 2);
    let (a, b, c, d, e) = fx.sim.skill_use(&mut fx.game, |w| {
        (
            BodyWorld::allied(w, pet, p),
            BodyWorld::allied(w, p, pet),
            BodyWorld::allied(w, wild, p),
            BodyWorld::allied(w, wild, wild),
            BodyWorld::allied(w, wild, wild2),
        )
    });
    assert!(a && b, "the pet and its owner");
    assert!(!c, "a monster without the link, other side");
    assert!(d, "the same unit");
    assert!(e, "two evil packs pair");
    fx.assert_clean();
}

#[test]
fn leash_owner_sets_and_clears_the_ai_control_owner() {
    use crate::monsters::ai::{AiControl, AiStore, UnitRef};
    use crate::skills::use_::bodies::{BodyEffect, BodyWorld};
    // Revive's leash owner, which the minion AI reads; none → GUID −1.
    let mut fx = Fx::new();
    let p = fx.spawn(UnitType::Player, 10, 10);
    let m = fx.spawn(UnitType::Monster, 11, 10);
    let guid = fx.sim.sys.units.get(p).unwrap().guid;
    let mut ai = AiStore::new();
    ai.entry(m).control = Some(AiControl::default());
    fx.sim.sys.hooks.ai = Some(ai);
    let owner = |fx: &mut Fx| {
        let ai = fx.sim.sys.hooks.ai.as_mut().unwrap();
        ai.control_mut(m).unwrap().owner
    };
    fx.sim.skill_use(&mut fx.game, |w| {
        w.effect(BodyEffect::LeashOwner { m, owner: Some(p) })
    });
    assert_eq!(
        owner(&mut fx),
        Some(UnitRef {
            ty: UnitType::Player,
            guid
        })
    );
    fx.sim.skill_use(&mut fx.game, |w| {
        w.effect(BodyEffect::LeashOwner { m, owner: None })
    });
    assert_eq!(owner(&mut fx), None);
    fx.assert_clean();
}

#[test]
fn resurrects_split_mode_request_sets_the_mode() {
    use crate::skills::use_::bodies::{BodyEffect, BodyWorld};
    use crate::skills::use_::UseWorld;
    use crate::skills::{SkillEntry, SkillUnits};
    // The build step (`ModeRequestBuild`) clears the used skill;
    // `resurrect_mode` sets it again; the send step (`ModeRequestSend`)
    // sets the mode, and its flag (m ≠ 14) clears the used skill once the
    // unit is in the requested mode.
    // A live monster asked for mode 12 (the fixture's monster class has no
    // other mode start that keeps its mode).
    let mut fx = Fx::new();
    let m = fx.spawn(UnitType::Monster, 11, 10);
    let used = SkillEntry {
        skill: 2,
        base: 1,
        owner_guid: -1,
        ..SkillEntry::default()
    };
    for (flag, kept) in [(true, false), (false, true)] {
        let (mode, skill) = fx.sim.skill_use(&mut fx.game, |w| {
            w.set_used_skill(m, Some(used));
            w.effect(BodyEffect::ModeRequestBuild { m, mode: 12 });
            assert!(w.used_skill(m).is_none(), "the builder clears it");
            w.set_used_skill(m, Some(used));
            w.effect(BodyEffect::ModeRequestSend { m, mode: 12, flag });
            (UseWorld::mode(w, m), w.used_skill(m))
        });
        assert_eq!(mode, 12, "flag {flag}: the mode is set");
        assert_eq!(skill.is_some(), kept, "flag {flag}");
        fx.sim.sys.units.get_mut(m).unwrap().mode = 1;
    }
    fx.assert_clean();
}

#[test]
fn a_monster_drawn_as_a_player_takes_the_charstats_walk_velocity() {
    // The base velocity follows the draw identity: the Shadow Warrior (monstats
    // `Velocity` 0) under a gfx state with `gfxtype` 2 and `gfxclass` 6
    // walks on the assassin's `WalkVelocity` (6); without the disguise
    // flag, its own monstats.
    use crate::units::record::flags2;
    const SHADOW: u32 = 119;
    let mut data = (*super::stat_data()).clone();
    data.states.set_gfx(vec![(SHADOW, 2, 6)]);
    let mut fx = Fx::with(Arc::new(data), skills());
    {
        let t = Arc::make_mut(&mut fx.sim.sys.hooks.tables);
        t.combat.monstats[0].velocity = 0;
        let mut ass = blank::<d2_data::tables::Charstats>();
        ass.walkvelocity = 6;
        t.combat.charstats.resize(7, ass.clone());
        t.combat.charstats[6] = ass;
    }
    let m = fx.spawn(UnitType::Monster, 12, 10);
    let v = |fx: &mut Fx| fx.sim.with(&mut fx.game, |_, v| v.velocity_base(m));
    assert_eq!(v(&mut fx), 0, "its own monstats");
    fx.sim.sys.stats.toggle_state(m, SHADOW, true);
    fx.sim.sys.units.get_mut(m).unwrap().flags2 |= flags2::DISGUISE;
    assert_eq!(v(&mut fx), 6, "the shown class's charstats");
    fx.assert_clean();
}

#[test]
fn leave_pack_clears_the_owner_and_the_leaders_entry() {
    use crate::monsters::ai::{AiControl, AiStore, UnitRef};
    use crate::skills::use_::bodies::helpers3::leave_pack;
    // The converted Fallen (pal-conversion-fallen) leaves
    // its leader's minion list and has no owner after.
    let mut fx = Fx::new();
    let lead = fx.spawn(UnitType::Monster, 10, 10);
    let a = fx.spawn(UnitType::Monster, 11, 10);
    let b = fx.spawn(UnitType::Monster, 12, 10);
    let g = |fx: &Fx, u| fx.sim.sys.units.get(u).unwrap().guid;
    let (gl, ga, gb) = (g(&fx, lead), g(&fx, a), g(&fx, b));
    let own = Some(UnitRef {
        ty: UnitType::Monster,
        guid: gl,
    });
    let mut ai = AiStore::new();
    ai.entry(lead).control = Some(AiControl {
        minion_owner: own,
        minions: vec![ga, gb],
        ..AiControl::default()
    });
    for m in [a, b] {
        ai.entry(m).control = Some(AiControl {
            minion_owner: own,
            ..AiControl::default()
        });
    }
    fx.sim.sys.hooks.ai = Some(ai);
    let ctl = |fx: &Fx, u| {
        fx.sim
            .sys
            .hooks
            .ai
            .as_ref()
            .unwrap()
            .control(u)
            .cloned()
            .unwrap()
    };
    fx.sim.skill_use(&mut fx.game, |w| leave_pack(w, a));
    assert_eq!(ctl(&fx, a).minion_owner, None, "a minion leaves");
    assert_eq!(ctl(&fx, lead).minions, vec![gb]);
    assert_eq!(ctl(&fx, b).minion_owner, own, "the other minion stays");
    fx.sim.skill_use(&mut fx.game, |w| leave_pack(w, lead));
    assert_eq!(ctl(&fx, b).minion_owner, None, "the pack is dissolved");
    assert!(ctl(&fx, lead).minions.is_empty());
    assert_eq!(
        ctl(&fx, lead).minion_owner,
        own,
        "the leader keeps its own owner"
    );
    fx.assert_clean();
}

/// A monster under a timed state-53 list with the Conversion remove
/// callback, its alignment 2; the list expires at frame 105.
fn converted_monster(fx: &mut Fx) -> UnitId {
    use crate::combat::range::STAT_ALIGNMENT;
    use crate::skills::use_::bodies::{callback, BodyWorld};
    let m = fx.spawn(UnitType::Monster, 13, 10);
    fx.game.frame = 100;
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| {
        w.cv.v.set_alignment(w.cv.game, m, 2);
        let l = w.alloc_list(2, 105, Some(m)).expect("list");
        w.set_list_state(l, 53);
        w.set_remove_callback(l, callback::CONVERSION);
        w.attach(m, l);
        w.state_on(m, 53, true);
    });
    fx.game
        .schedule_event(m, u32::from(event::REMOVE_STATE), 105, None, 0, 0)
        .unwrap();
    assert_eq!(fx.stat(m, STAT_ALIGNMENT), 2);
    m
}

#[test]
fn an_expired_conversion_list_runs_its_callback_and_resets_the_alignment() {
    use crate::combat::range::STAT_ALIGNMENT;
    let mut fx = Fx::new();
    let m = converted_monster(&mut fx);
    for _ in 100..106 {
        fx.frame();
    }
    assert!(!fx.sim.sys.stats.has_state(m, 53));
    assert_eq!(fx.stat(m, STAT_ALIGNMENT), 0);
    assert!(fx.sim.sys.hooks.removed_lists.is_empty());
    fx.assert_clean();
}

#[test]
fn a_body_detach_runs_the_conversion_callback_once() {
    use crate::combat::range::STAT_ALIGNMENT;
    use crate::skills::use_::bodies::BodyWorld;
    let mut fx = Fx::new();
    let m = converted_monster(&mut fx);
    let game = &mut fx.game;
    fx.sim.skill_use(game, |w| {
        let l = w.state_list(m, 53).expect("list");
        w.detach_free(m, l);
    });
    assert_eq!(fx.stat(m, STAT_ALIGNMENT), 0);
    // Not queued again for the expiry runner.
    assert!(fx.sim.sys.hooks.removed_lists.is_empty());
    fx.assert_clean();
}
