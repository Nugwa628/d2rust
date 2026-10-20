//! Tests of the skill handlers through the real host frame (drain → tick →
//! flush) on the wired sim (`ActionSim`, the skill use pipeline's
//! `UseView`): each handled id with its reference vectors, accepted and
//! refused, and the exact bytes the client receives. The units stand in
//! a real field room (Cold Plains of the waypoint tests' DRLG).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use d2_data::bin::BinTable;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Charstats, Experience, Itemstatcost, Record, Skilldesc, Skills};
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::combat::CombatTables;
use d2_sim::game::Game;
use d2_sim::rng::Seed;
use d2_sim::skills::use_::{ModeTarget, ServerMsg, UseState};
use d2_sim::skills::{SkillEntry, SkillTables, LEVEL_CAP_114D};
use d2_sim::stats::{StatData, StatTable};
use d2_sim::units::anim::AnimError;
use d2_sim::units::hooks::{MonsterInfo, UnitData};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::modes::UnitError;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables, Pending, WiringError};
use d2_sim::wiring::interaction::UseRest;

use super::wired::WiredSkills;
use super::LearnRest;
use crate::adapters::handlers::world::tests::waypoints::{field_drlg, field_room};
use crate::adapters::handlers::world::{ActionWorld, Outbox};
use crate::adapters::{PlayerData, PlayerFields, ProtoSizes, SimGame, UnitFacts};
use crate::dispatch::Outcome;
use crate::host::{Handled, Host};
use crate::seams::{ClientId, Clock, MessageSink, PlayerGate, Pos, ResultCode, SessionHandler};

// ---- fixture -----------------------------------------------------------------------------

#[derive(Default)]
struct NoSession;
impl SessionHandler for NoSession {
    fn system_message(&mut self, _: ClientId, _: &[u8], _: usize, _: &mut dyn MessageSink) {}
}
struct Clock0(u32);
impl Clock for Clock0 {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}

/// A synthetic itemstatcost: 359 stats, no ops, no shifts, no minimums
/// (the handlers' stats are plain base values), fixed up.
fn stat_data() -> Arc<StatData> {
    let (n, size) = (359, Itemstatcost::SIZE);
    let mut records = vec![0u8; n * size];
    for (s, r) in records.chunks_mut(size).enumerate() {
        for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
            r[o..o + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
        r[0..2].copy_from_slice(&(s as u16).to_le_bytes());
    }
    let mut t = BinTable {
        name: "itemstatcost".into(),
        source: "synthetic".into(),
        count: n,
        record_size: size,
        records,
    };
    stat_ops(&mut t);
    Arc::new(StatData {
        stats: StatTable::from_fixed(&t).expect("itemstatcost"),
        ..StatData::default()
    })
}

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// A skills record with no formulas and no required skills.
fn skill_rec() -> Skills {
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
        &mut s.delay,
        &mut s.perdelay,
    ] {
        *f = 0xFFFF_FFFF;
    }
    s.skilldesc = 0xFFFF;
    s.charclass = 0xFF;
    s.reqskill1 = 0xFFFF;
    s.reqskill2 = 0xFFFF;
    s.reqskill3 = 0xFFFF;
    s.itypea1 = 0xFFFF;
    s.srvmissile = 0xFFFF;
    s.intown = true;
    s.ingame = true;
    s
}

/// Skill ids of the synthetic table.
const ATTACK: i32 = 0;
/// Multiple Shot as this table gives it: srvst 52; mana 4, +1, shift 8.
const MULTI: i32 = 1;
/// Might: aura, immediate, perdelay 50, srvdo 53.
const MIGHT: i32 = 2;
/// A learnable skill: max level 3.
const LEARN: i32 = 3;

/// Start slot of the synthetic start skills: srvst 52 (Emerge: unit
/// flags |= 0xE, return 1) stands in for Multiple Shot's srvst 4, whose
/// body (ammunition) needs items; every filled start slot has a body, so
/// the start is seen through its flags. The do slot 53 (filled,
/// `unreferenced`, no body) stands in for Might's 65, so the fake's seam
/// answers.
fn skills() -> SkillTables {
    let mut v: Vec<Skills> = (0..4).map(|_| skill_rec()).collect();
    let m = &mut v[MULTI as usize];
    (m.srvstfunc, m.mana, m.lvlmana, m.manashift) = (52, 4, 1, 8);
    let m = &mut v[MIGHT as usize];
    (m.aura, m.immediate, m.perdelay, m.srvdofunc, m.aurastate) = (true, true, 0, 53, 33);
    v[LEARN as usize].maxlvl = 3;
    SkillTables {
        skills: v,
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: Vec::new(),
        // Formula 0: the constant 50.
        skills_code: vec![0x07, 50, 0x00],
        miss_code: Vec::new(),
        level_cap: LEVEL_CAP_114D,
        stat_count: 359,
    }
}

/// The vitals constants: `charstats` (Sorceress, Barbarian rows used)
/// and `experience.txt` MaxLvl 99.
fn vitals() -> VitalsTables {
    let rows: [[u8; 13]; 7] = [
        [20, 25, 15, 20, 84, 30, 8, 4, 6, 12, 4, 6, 5],
        [10, 25, 35, 10, 74, 30, 4, 4, 8, 8, 4, 8, 5],
        [15, 25, 25, 15, 79, 30, 6, 4, 8, 8, 4, 8, 5],
        [25, 20, 15, 25, 89, 30, 8, 4, 6, 12, 4, 6, 5],
        [30, 20, 10, 25, 92, 30, 8, 4, 4, 16, 4, 4, 5],
        [15, 20, 20, 25, 84, 30, 6, 4, 8, 8, 4, 8, 5],
        [20, 20, 25, 20, 95, 30, 8, 5, 6, 12, 5, 7, 5],
    ];
    let charstats = rows
        .iter()
        .map(|r| {
            let mut c: Charstats = blank();
            (c.str, c.dex, c.int, c.vit, c.stamina, c.hpadd) = (r[0], r[1], r[2], r[3], r[4], r[5]);
            (c.lifeperlevel, c.staminaperlevel, c.manaperlevel) = (r[6], r[7], r[8]);
            (c.lifepervitality, c.staminapervitality, c.manapermagic) = (r[9], r[10], r[11]);
            c.statperlevel = r[12];
            c
        })
        .collect();
    let mut row: Experience = blank();
    row.amazon = 99;
    VitalsTables {
        charstats,
        experience: vec![row],
    }
}

/// What the unspecified seams answer, staged by the test, and a log of
/// the calls that change something.
#[derive(Default)]
struct Inner {
    list: Vec<SkillEntry>,
    left: Option<SkillEntry>,
    right: Option<SkillEntry>,
    used: Option<SkillEntry>,
    /// Entry mode by skill (skill entry +8).
    modes: BTreeMap<i32, u32>,
    /// `use_state` by skill (default usable).
    states: BTreeMap<i32, UseState>,
    class_skills: Vec<i32>,
    log: Vec<String>,
}

/// The action wiring's `Pending` value: the seams without a provider
/// (`Pending`'s defaults, the skill use pipeline's `UseRest`, the
/// skill-point calls), answered from [`Inner`]; the test keeps a handle.
#[derive(Clone, Default)]
struct Book(Arc<Mutex<Inner>>);

impl Book {
    fn of(i: Inner) -> Self {
        Book(Arc::new(Mutex::new(i)))
    }
    fn get(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap()
    }
}

impl Pending for Book {
    fn skill_list(&self, _: UnitId) -> Vec<SkillEntry> {
        self.get().list.clone()
    }
    fn used_skill(&self, _: UnitId) -> Option<SkillEntry> {
        self.get().used
    }
    fn stats_refresh(&mut self, _: UnitId) {
        self.get().log.push("refresh".into());
    }
}

impl Outbox for Book {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        Vec::new()
    }
}

/// The narrowest answer wherever the test stages nothing. The message
/// path's player data, positions, reach and sends are the server's
/// (`skills::world::World`), so those calls are never reached here.
impl UseRest for Book {
    fn send(&mut self, _: UnitId, _: ServerMsg) {}
    fn has_player_data(&self, _: UnitId) -> bool {
        false
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
        false
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
        self.get().left
    }
    fn right_skill(&self, _: UnitId) -> Option<SkillEntry> {
        self.get().right
    }
    fn set_left_skill(&mut self, _: UnitId, e: SkillEntry) {
        self.get().log.push(format!("left {}", e.skill));
        self.get().left = Some(e);
    }
    fn set_right_skill(&mut self, _: UnitId, e: SkillEntry) {
        self.get().log.push(format!("right {}", e.skill));
        self.get().right = Some(e);
    }
    fn find_entry(&self, _: UnitId, skill: i32) -> Option<SkillEntry> {
        self.get().list.iter().copied().find(|e| e.skill == skill)
    }
    fn find_entry_owned(&self, _: UnitId, skill: i32, owner: i32) -> Option<SkillEntry> {
        self.get()
            .list
            .iter()
            .copied()
            .find(|e| e.skill == skill && e.owner_guid == owner)
    }
    fn owns_skill(&self, _: UnitId, _: i32) -> bool {
        false
    }
    fn set_used_skill(&mut self, _: UnitId, e: Option<SkillEntry>) {
        self.get().used = e;
    }
    fn used_skill_flags(&self, _: UnitId) -> u32 {
        0
    }
    fn set_used_skill_flags(&mut self, _: UnitId, _: u32) {}
    fn entry_mode(&self, _: UnitId, e: &SkillEntry) -> u32 {
        self.get().modes.get(&e.skill).copied().unwrap_or(0)
    }
    fn attack_param4(&self, _: UnitId) -> i32 {
        0
    }
    fn set_attack_param4(&mut self, _: UnitId, _: i32) {}
    fn use_state(&mut self, _: UnitId, e: &SkillEntry) -> UseState {
        self.get()
            .states
            .get(&e.skill)
            .copied()
            .unwrap_or(UseState::Usable)
    }
    fn shapeshifted(&self, _: UnitId) -> bool {
        false
    }
    fn consume_charges(&mut self, _: UnitId, _: &SkillEntry) -> bool {
        false
    }
    fn pay_life(&mut self, _: UnitId, _: i32) -> bool {
        false
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
    fn target(&self, _: UnitId) -> Option<UnitId> {
        None
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
        None
    }
    fn line_clear(&self, _: UnitId, _: (i32, i32), _: u32) -> bool {
        false
    }
    fn set_aura_state(&mut self, _: UnitId, _: u16, _: i32, _: i32) {}
    fn srvst(&mut self, index: u16, _: UnitId, skill: i32, lvl: i32) -> i32 {
        self.get().log.push(format!("srvst {index} {skill} {lvl}"));
        1
    }
    fn srvdo(&mut self, i: u16, _: UnitId, s: i32, l: i32, c: bool, it: bool, a: bool) -> i32 {
        self.get()
            .log
            .push(format!("srvdo {i} {s} {l} {c} {it} {a}"));
        1
    }
}

impl LearnRest for Book {
    fn is_class_skill(&self, _: UnitId, skill: i32) -> bool {
        self.get().class_skills.contains(&skill)
    }
    fn add_skill_level(&mut self, _: UnitId, skill: i32, cost: i32) {
        self.get().log.push(format!("addskill {skill} {cost}"));
    }
    fn after_skill_point(&mut self, _: UnitId) {
        self.get().log.push("after".into());
    }
}

const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

type Wired = SimGame<ActionSim<Book>, ActionWorld<WiredSkills>>;

/// A host with a player (class `class`, mode NU, at (100, 100)) for
/// client 0, a monster at (120, 100) and an item on the ground at
/// (101, 100), all in one field room, on the wired sim with [`Book`] as
/// the unspecified seams.
struct Fx {
    host: Host<Wired, ProtoSizes, NoSession, Clock0>,
    player: UnitId,
    monster: UnitId,
    item: UnitId,
    book: Book,
    errors: Vec<WiringError>,
}

impl Fx {
    fn new(class: u32, inner: Inner) -> Self {
        let book = Book::of(inner);
        let tables = ActionTables {
            missiles: Vec::new(),
            skills: skills(),
            combat: CombatTables {
                charstats: Vec::new(),
                difficultylevels: Vec::new(),
                monstats: Vec::new(),
                monstats2: Vec::new(),
                hitclass: Vec::new(),
            },
            levels: Vec::new(),
            skill_modes: Vec::new(),
            overlay_count: 0,
            monequip: Vec::new(),
            arena: Vec::new(),
        };
        let mut hooks = ActionHooks::new(
            Arc::new(tables),
            field_drlg(),
            Seed::init_low(1234),
            book.clone(),
        );
        hooks.vitals = Some(Arc::new(vitals()));
        let data = UnitData {
            monsters: vec![MonsterInfo {
                mode_chart: false,
                enabled: true,
                aidel: [15; 3],
                moves: 0,
            }],
            ..UnitData::default()
        };
        let mut events = ActionSim::new(stat_data(), data, hooks);
        let mut game = Game::new();
        let room = field_room(&mut events, &mut game);
        let mut alloc = |ty, class| {
            let req = AllocRequest {
                ty,
                class,
                room: Some(room),
                add: true,
                fixed_guid: None,
                mode: 1,
                allied: ty == UnitType::Player,
            };
            events
                .with(&mut game, |g, v| v.allocate(g, &req, 20, 20))
                .expect("allocated")
        };
        let player = alloc(UnitType::Player, class);
        let monster = alloc(UnitType::Monster, 0);
        let item = alloc(UnitType::Item, 0);
        // Players are allocated in mode 0; neutral.
        events.sys.units.get_mut(player).unwrap().mode = 1;
        let world = ActionWorld {
            skills: WiredSkills::default(),
            ..ActionWorld::default()
        };
        let mut sim = SimGame::with_world(game, events, world);
        sim.join(0, Some(player), Some(room), client_state::IN_GAME)
            .unwrap();
        sim.set_player(
            player,
            PlayerFields {
                gate: ALIVE,
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        let at = |x, y| UnitFacts {
            act: 0,
            pos: Pos { x, y },
            owner: None,
        };
        sim.set_unit(player, at(100, 100));
        sim.set_unit(monster, at(120, 100));
        sim.set_unit(item, at(101, 100));
        let mut host = Host::new(sim, ProtoSizes, NoSession, Clock0(1000));
        host.connect(0);
        Fx {
            host,
            player,
            monster,
            item,
            book,
            errors: Vec::new(),
        }
    }

    fn book(&self) -> MutexGuard<'_, Inner> {
        self.book.get()
    }

    fn unsent(&self) -> Vec<(ClientId, ServerMsg)> {
        self.host.game.world.skills.unsent.clone()
    }

    fn guid(&self, u: UnitId) -> u32 {
        self.host.game.game.lists.unit(u).unwrap().guid
    }

    fn stat(&mut self, s: u16) -> i32 {
        let p = self.player;
        let g = &mut self.host.game;
        g.events.with(&mut g.game, |_, v| v.stat(p, s))
    }

    fn set_stats(&mut self, values: &[(u16, i32)]) {
        let p = self.player;
        let g = &mut self.host.game;
        g.events.with(&mut g.game, |_, v| {
            for &(s, x) in values {
                v.set_base(p, s, x);
            }
        });
    }

    /// The caster's unit flags.
    fn flags(&mut self) -> &mut u32 {
        &mut self
            .host
            .game
            .events
            .sys
            .units
            .get_mut(self.player)
            .unwrap()
            .flags
    }

    fn mode(&self) -> u32 {
        self.host
            .game
            .events
            .sys
            .units
            .get(self.player)
            .unwrap()
            .mode
    }

    /// Sends `msgs` and runs one host frame; the result codes, then what
    /// the client receives after the frame's flush.
    fn frame(&mut self, msgs: &[&[u8]]) -> (Vec<ResultCode>, Vec<Vec<u8>>) {
        for m in msgs {
            self.host.send_game(0, m).unwrap();
        }
        self.host.clock.0 += 40;
        let r = self.host.frame().unwrap();
        let codes = r
            .messages
            .iter()
            .map(|h| match h.handled {
                Handled::Game(Outcome::Dispatched(c)) => c,
                other => panic!("not dispatched: {other:?}"),
            })
            .collect();
        let sys = &mut self.host.game.events.sys;
        assert!(sys.errors.is_empty(), "{:?}", sys.errors);
        self.errors.append(&mut sys.hooks.errors);
        (codes, self.host.receive(0))
    }

    /// The wiring errors since the last call.
    fn take_errors(&mut self) -> Vec<WiringError> {
        std::mem::take(&mut self.errors)
    }

    fn pierce(&mut self) -> i32 {
        self.stat(328)
    }
}

fn entry(skill: i32, base: i32) -> SkillEntry {
    SkillEntry {
        skill,
        base,
        owner_guid: -1,
        ..SkillEntry::default()
    }
}

fn point(id: u8, x: u16, y: u16) -> Vec<u8> {
    let mut m = vec![id];
    m.extend(x.to_le_bytes());
    m.extend(y.to_le_bytes());
    m
}

fn unit_msg(id: u8, ty: u32, guid: u32) -> Vec<u8> {
    let mut m = vec![id];
    m.extend(ty.to_le_bytes());
    m.extend(guid.to_le_bytes());
    m
}

use ResultCode::*;
const NONE: Vec<Vec<u8>> = Vec::new();

/// What one mode start reports on the wired sim today: the action
/// wiring routes no AnimData record, so the frame-event schedule fails
/// after the mode set.
fn no_anim_record() -> WiringError {
    WiringError::Unit(UnitError::Anim(AnimError::NoRecord))
}

/// A caster with Multiple Shot L10 as the right skill (mode SC) and
/// 4,000 mana.
fn caster() -> Fx {
    let multi = entry(MULTI, 10);
    let book = Inner {
        list: vec![entry(ATTACK, 1), multi],
        left: Some(entry(ATTACK, 1)),
        right: Some(multi),
        modes: [(ATTACK, 7), (MULTI, 10)].into(),
        ..Inner::default()
    };
    let mut fx = Fx::new(0, book);
    fx.set_stats(&[(8, 4000), (12, 1)]);
    fx
}

// ---- 0x05–0x11 -------------------------------------------------------------------------

/// The point-skill rules, the mode start and the Multiple Shot vector
/// (3,328 charged at start), through one host frame.
#[test]
fn right_skill_at_point_starts_and_charges_at_start() {
    let mut fx = caster();
    fx.host.game.game.coverage.enabled = true;
    *fx.flags() &= !0xE;
    let (codes, recv) = fx.frame(&[&point(0x0C, 110, 90)]);
    assert_eq!((codes, recv), (vec![Done], NONE));
    assert_eq!(fx.pierce(), 1);
    assert_eq!(fx.mode(), 10, "SC started (units.md §4.1)");
    assert_eq!(fx.take_errors(), [no_anim_record()]);
    assert_eq!(fx.stat(8), 4000 - 3328);
    assert_eq!(
        fx.host.game.game.coverage.rows[&("skills_cast", MULTI as u32)],
        1
    );
    assert_eq!(fx.book().used, Some(entry(MULTI, 10)));
    // The start body ran (srvst 52: flags |= 0xE), not the seam.
    assert_eq!(*fx.flags() & 0xE, 0xE);
    assert!(fx.book().log.is_empty());
    // The point validator stored the frame.
    let p = fx.player;
    let data = fx.host.game.player_fields(p).unwrap().data;
    assert_eq!(data, Some(PlayerData { last_accept: 0 }));
    assert!(fx.unsent().is_empty());
}

/// Refusals: no right skill → 3 without `pierce_idx`; out of range → 1
/// (the shared parser, before the handler); a dead player → 0 (gate).
#[test]
fn skill_message_refusals() {
    let mut fx = caster();
    fx.book().right = None;
    let far = point(0x0C, 151, 100);
    let (codes, recv) = fx.frame(&[&point(0x0C, 110, 90), &far]);
    assert_eq!((codes, recv), (vec![Malformed, Refused], NONE));
    assert_eq!(fx.pierce(), 0);
    let p = fx.player;
    fx.host.game.set_player(
        p,
        PlayerFields {
            gate: PlayerGate { mode: 17, ..ALIVE },
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    fx.book().right = Some(entry(MULTI, 10));
    let (codes, _) = fx.frame(&[&point(0x0C, 110, 90)]);
    assert_eq!(codes, vec![Done]);
    assert_eq!((fx.pierce(), fx.mode()), (0, 1));
    assert!(fx.take_errors().is_empty());
}

/// No mana → 0x5A `5A 0E 01` + 37 zero bytes to the caster, result 0 and
/// `pierce_idx` + 1.
#[test]
fn no_mana_sends_cant_do_and_counts() {
    let mut fx = caster();
    fx.host.game.game.coverage.enabled = true;
    fx.book().states.insert(MULTI, UseState::NoMana);
    let (codes, recv) = fx.frame(&[&point(0x0C, 110, 90)]);
    assert_eq!((codes, recv), (vec![Done], NONE));
    assert_eq!((fx.pierce(), fx.mode(), fx.stat(8)), (1, 1, 4000));
    assert!(!fx
        .host
        .game
        .game
        .coverage
        .rows
        .contains_key(&("skills_cast", MULTI as u32)));
    assert!(fx.unsent().is_empty());
    // Queued at the message, flushed with the next frame's sends.
    let (_, recv) = fx.frame(&[]);
    assert_eq!(recv, vec![super::wired::CANT_DO.to_vec()]);
    assert!(fx.take_errors().is_empty());
}

/// Hold forms and the left skill (0x05): 0 whatever the use
/// did; one `pierce_idx` per message. 0x0B does nothing.
#[test]
fn hold_and_left_forms() {
    let mut fx = caster();
    // Attack is not affordable here: the use fails, the result is 0.
    fx.book().states.insert(ATTACK, UseState::Cooldown);
    let (codes, recv) = fx.frame(&[&point(0x08, 110, 90), &point(0x05, 110, 90), &[0x0B]]);
    assert_eq!((codes, recv), (vec![Done, Done, Done], NONE));
    assert_eq!((fx.pierce(), fx.mode()), (2, 1));
    assert!(fx.take_errors().is_empty());
    // 0x0F: the right hold form starts like 0x0C.
    let (codes, _) = fx.frame(&[&point(0x0F, 110, 90)]);
    assert_eq!(codes, vec![Done]);
    assert_eq!((fx.pierce(), fx.mode()), (3, 10));
    assert_eq!(fx.take_errors(), [no_anim_record()]);
}

/// Unit forms: 0x0D on a monster uses now (range "none"); 0x0E on
/// an item without `TargetItem` → 2 inside, result 0; a
/// missing unit → 1 and a bad type → 2 from the shared parser.
#[test]
fn unit_forms() {
    let mut fx = caster();
    let (m, i) = (fx.guid(fx.monster), fx.guid(fx.item));
    let (codes, recv) = fx.frame(&[
        &unit_msg(0x0E, 4, i),
        &unit_msg(0x0D, 1, m + 50),
        &unit_msg(0x0D, 6, m),
    ]);
    assert_eq!((codes, recv), (vec![Done, Refused, Invalid], NONE));
    assert_eq!((fx.pierce(), fx.mode()), (1, 1));
    assert!(fx.take_errors().is_empty());
    let (codes, recv) = fx.frame(&[&unit_msg(0x0D, 1, m)]);
    assert_eq!((codes, recv), (vec![Done], NONE));
    assert_eq!((fx.pierce(), fx.mode()), (2, 10));
    assert_eq!(fx.take_errors(), [no_anim_record()]);
    let _ = fx.monster;
}

// ---- 0x3C -------------------------------------------------------------------------------

/// Select skill: Might assigned at frame 1234 runs its do now and
/// schedules type 8 at 1251; the gate is none (a dead player may
/// select); unknown or level-0 entries → 3.
#[test]
fn select_skill_might_and_refusals() {
    let book = Inner {
        list: vec![entry(ATTACK, 1), entry(MIGHT, 1), entry(LEARN, 0)],
        ..Inner::default()
    };
    let mut fx = Fx::new(3, book);
    let p = fx.player;
    fx.host.game.game.frame = 1234;
    fx.host.game.set_player(
        p,
        PlayerFields {
            gate: PlayerGate { mode: 17, ..ALIVE },
            data: None,
        },
    );
    let sel = |skill: u32, owner: i32| {
        let mut m = vec![0x3C];
        m.extend(skill.to_le_bytes());
        m.extend(owner.to_le_bytes());
        m
    };
    let (codes, recv) = fx.frame(&[
        &sel(MIGHT as u32, -1),
        &sel(ATTACK as u32 | 0x8000_0000, -1),
        &sel(9, -1),
        &sel(LEARN as u32, -1),
        &sel(MIGHT as u32, 7),
    ]);
    assert_eq!(codes, vec![Done, Done, Malformed, Malformed, Malformed]);
    // Each accepted selection is answered with S→C 0x23: type 0, the
    // player's GUID, hand (0 right, 1 left), skill, owner. The refused ones send nothing.
    let guid = fx.guid(p);
    let reply = |hand: u8, skill: i32| {
        let mut m = vec![0x23, 0];
        m.extend(guid.to_le_bytes());
        m.push(hand);
        m.extend((skill as u16).to_le_bytes());
        m.extend(u32::MAX.to_le_bytes());
        m
    };
    assert_eq!(recv, NONE, "queued: the buffers flush with the next frame");
    assert_eq!(
        fx.book().log,
        ["right 2", "srvdo 53 2 1 true false false", "left 0"]
    );
    let g = &fx.host.game.game;
    let timers: Vec<_> = g
        .timers
        .unit_timers(p)
        .into_iter()
        .filter_map(|t| Some((g.timers.event(t)?, g.timers.expire(t)?)))
        .collect();
    assert_eq!(timers, vec![((8, u32::MAX, 0), 1251)]);
    assert_eq!(fx.pierce(), 0);
    let (_, recv) = fx.frame(&[]);
    assert_eq!(recv, vec![reply(0, MIGHT), reply(1, ATTACK)]);
    assert!(fx.take_errors().is_empty());
}

/// The reply bytes of [`set_skill_reply`]: left is bit 31 of the skill
/// word, the owner is copied, a short message gives none.
#[test]
fn the_set_skill_reply_follows_the_select_message() {
    use super::set_skill_reply;
    let mut m = vec![0x3C];
    m.extend((36u32 | 0x8000_0000).to_le_bytes());
    m.extend(7u32.to_le_bytes());
    assert_eq!(
        set_skill_reply(0x0102_0304, &m),
        Some([0x23, 0, 4, 3, 2, 1, 1, 36, 0, 7, 0, 0, 0])
    );
    m[1..5].copy_from_slice(&54u32.to_le_bytes());
    m[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        set_skill_reply(1, &m),
        Some([0x23, 0, 1, 0, 0, 0, 0, 54, 0, 255, 255, 255, 255])
    );
    assert_eq!(set_skill_reply(1, &m[..8]), None);
}

// ---- 0x3A -------------------------------------------------------------------------------

/// Add-stat-point vectors on a Barbarian: `3A 03 04` with 3 points →
/// three spends, then 2; +10 vitality → max life +10,240, max stamina
/// +2,560; range checks → 3; ids 4–15 → 2; size → 3.
#[test]
fn add_stat_point_vectors() {
    let mut fx = Fx::new(4, Inner::default());
    let base = [(3, 25), (6, 14080), (7, 14080), (10, 23552), (11, 23552)];
    fx.set_stats(&base);
    fx.set_stats(&[(4, 3)]);
    let (codes, recv) = fx.frame(&[&[0x3A, 0x03, 0x04]]);
    assert_eq!((codes, recv), (vec![Invalid], NONE));
    let got: Vec<_> = [3, 4, 6, 7, 10, 11].map(|s| fx.stat(s)).to_vec();
    assert_eq!(
        got,
        [28, 0, 14080 + 3072, 14080 + 3072, 23552 + 768, 23552 + 768]
    );

    fx.set_stats(&base);
    fx.set_stats(&[(4, 10)]);
    let (codes, _) = fx.frame(&[&[0x3A, 0x03, 0x09]]);
    assert_eq!(codes, vec![Done]);
    let got: Vec<_> = [3, 4, 7, 11].map(|s| fx.stat(s)).to_vec();
    assert_eq!(got, [35, 0, 14080 + 10240, 23552 + 2560]);

    fx.set_stats(&[(4, 5), (0, 30)]);
    let (codes, _) = fx.frame(&[
        &[0x3A, 0x10, 0x00],
        &[0x3A, 0x00, 0x64],
        &[0x3A, 0x04, 0x00],
        &[0x3A, 0x00, 0x00],
    ]);
    assert_eq!(codes, vec![Malformed, Malformed, Invalid, Done]);
    assert_eq!((fx.stat(0), fx.stat(4)), (31, 4));
    assert_eq!(fx.book().log, ["refresh"]);
    assert!(fx.take_errors().is_empty());
}

/// Sorceress, +10 energy → max mana +5,120.
#[test]
fn add_energy_vector() {
    let mut fx = Fx::new(1, Inner::default());
    fx.set_stats(&[(1, 35), (8, 8960), (9, 8960), (4, 10)]);
    let (codes, recv) = fx.frame(&[&[0x3A, 0x01, 0x09]]);
    assert_eq!((codes, recv), (vec![Done], NONE));
    let got: Vec<_> = [1, 4, 8, 9].map(|s| fx.stat(s)).to_vec();
    assert_eq!(got, [45, 0, 8960 + 5120, 8960 + 5120]);
    assert!(fx.take_errors().is_empty());
}

// ---- 0x3B -------------------------------------------------------------------------------

/// Add skill point: not a class skill → 3; bad id → 2; at max level →
/// 2; spend (cost 1, `skpoints` empty) then the follow-up; without points
/// no level, the follow-up still runs.
#[test]
fn add_skill_point_vectors() {
    let book = Inner {
        list: vec![entry(LEARN, 1)],
        class_skills: vec![LEARN, MULTI],
        ..Inner::default()
    };
    let mut fx = Fx::new(1, book);
    fx.set_stats(&[(12, 1), (5, 1)]);
    let sk = |s: u16| {
        let mut m = vec![0x3B];
        m.extend(s.to_le_bytes());
        m
    };
    let (codes, recv) = fx.frame(&[&sk(ATTACK as u16), &sk(40), &sk(LEARN as u16)]);
    assert_eq!((codes, recv), (vec![Malformed, Invalid, Done], NONE));
    assert_eq!(fx.book().log, ["addskill 3 1", "after"]);
    fx.book().log.clear();
    // The spend added the skill to the player's own list (`skill_lists`,
    // the one player list); this fixture's seam list stands in again.
    fx.host.game.events.hooks().skill_lists.clear();
    fx.book().list = vec![entry(LEARN, 3)];
    fx.set_stats(&[(5, 0), (12, 5)]);
    // (Not 0x3B LEARN first: the client's duplicate filter
    // would drop a repeat of the last message.)
    let (codes, _) = fx.frame(&[&sk(MULTI as u16), &sk(LEARN as u16)]);
    assert_eq!(codes, vec![Done, Invalid]);
    assert_eq!(fx.book().log, ["after"]);
    assert!(fx.take_errors().is_empty());
}

// ---- the id table ------------------------------------------------------------------------

const CLIENT_TSV: &str = include_str!("../../../../../../tables/sim/client-messages.tsv");

/// Rows of [`super::IDS`] that disagree with `client-messages.tsv`: the
/// name, and `kind` = handler for every handled id.
fn ids_vs_tsv(tsv: &str) -> Vec<String> {
    let rows: BTreeMap<u8, (String, String)> = tsv
        .lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            let id = u8::from_str_radix(c.first()?.trim_start_matches("0x"), 16).ok()?;
            Some((id, (c.get(1)?.to_string(), c.get(6)?.to_string())))
        })
        .collect();
    let mut bad = Vec::new();
    for &(id, name, _, status) in super::IDS {
        match rows.get(&id) {
            Some((n, k)) if n == name && (status != super::Status::Handled || k == "handler") => {}
            other => bad.push(format!("{id:#04x} {name}: {other:?}")),
        }
    }
    bad
}

#[test]
fn ids_match_client_tsv() {
    assert_eq!(ids_vs_tsv(CLIENT_TSV), Vec::<String>::new());
}

/// M08: a renamed row and a row whose kind is no longer `handler` are
/// both reported, and nothing else.
#[test]
fn ids_check_reports_perturbations() {
    let renamed = CLIENT_TSV.replace("\tAddSkillPoint\t", "\tAddSkillPointX\t");
    let bad = ids_vs_tsv(&renamed);
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].starts_with("0x3b AddSkillPoint"));
    let stubbed = CLIENT_TSV.replace("0x0054C0C0\thandler", "0x0054C0C0\tstub0");
    assert!(ids_vs_tsv(&stubbed).is_empty(), "0x40 is not in the table");
    let stubbed = CLIENT_TSV.replace("0x0054BE70\thandler", "0x0054BE70\tstub0");
    let bad = ids_vs_tsv(&stubbed);
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].starts_with("0x3c SelectSkill"));
}
