//! Tests for the animation schedule on AnimData records, the kill after a missile
//! hit (death mode with its target, experience) and the dead monster's
//! drop as real item units in its room.

use d2_data::fixup::maps::EquivMatrix;
use d2_data::tables::{Experience, Itemratio, Itemtypes, Monstats};
use d2_formats::animdata::{self, AnimData, AnimRecord};

use super::*;
use crate::combat::vitals::VitalsTables;
use crate::drlg::collision::bits;
use crate::items::tables::ItemRec;
use crate::items::{ty, ItemTables};
use crate::missiles::{create_missile, param_flags, unit_flag, MissileParams};
use crate::stats::stat as st;
use crate::tick::events::event;
use crate::treasure::{ItemData, TcEntry, TreasureClass, TreasureClasses};
use crate::units::hooks::Sim;
use crate::units::modes::{self, monster_mode};
use crate::wiring::economy::{
    monster_death_drop, DeathDrops, DropSpot, DropTables, FreeSpot, GameFields,
};

const TOHIT: u16 = 19;
const LEVEL_STAT: u16 = 12;
const EXPERIENCE: u16 = 13;
const MINDAMAGE: u16 = 21;
const MAXDAMAGE: u16 = 22;
const GOLD_STAT: u16 = 14;

const CAST: &[u8; 8] = b"SOSCHTH\0";
const DEATH: &[u8; 8] = b"M0DTHTH\0";

/// The fixture's AnimData: the cast, 8 frames at speed 256 with event 2
/// on frame 4; the death, 4 frames, no events.
fn anim_data() -> AnimData {
    let mut a = AnimData {
        buckets: vec![Vec::new(); animdata::BUCKETS],
    };
    for (name, frames, event) in [(CAST, 8, Some(4)), (DEATH, 4, None)] {
        let mut events = [0u8; animdata::EVENTS];
        if let Some(i) = event {
            events[i] = 2;
        }
        let len = name.iter().position(|&b| b == 0).unwrap();
        a.buckets[animdata::hash(&name[..len])].push(AnimRecord {
            name: *name,
            frames,
            speed: 256,
            events,
        });
    }
    a
}

fn with_anim(fx: &mut Fx) {
    let h = fx.sim.hooks();
    h.anim_data = Some(Arc::new(anim_data()));
    h.x.names.insert((UnitType::Player, 10), *CAST);
    h.x.names.insert((UnitType::Monster, 0), *DEATH);
}

/// Starts the mode's animation, then schedules it (main form), on the unit.
fn animate(fx: &mut Fx, u: UnitId, mode: u32) {
    let s = &mut fx.sim.sys;
    let mut sim = Sim {
        game: &mut fx.game,
        units: &mut s.units,
        stats: &mut s.stats,
        data: &s.data,
    };
    modes::set_mode(&mut sim, &mut s.hooks, u, mode).unwrap();
    modes::animate(&mut sim, &mut s.hooks, u).unwrap();
}

#[test]
fn mode_start_schedules_from_the_animdata_record_of_the_composed_name() {
    let mut fx = Fx::new();
    with_anim(&mut fx);
    let p = fx.spawn(UnitType::Player, 1, fx.a, 10, 10);
    fx.game.frame = 100;
    animate(&mut fx, p, 10);
    // s = 256, F = 8 · 256: the event byte 2 of frame 4 → event 0
    // at f + 4, args (2, 0); the end at f + 8.
    assert_eq!(
        fx.timers(p),
        [(event::MODE_CHANGE, 104), (event::END_ANIM, 108)]
    );
    let r = fx.sim.sys.units.get(p).unwrap();
    assert_eq!((r.anim.frame_count, r.anim.speed), (8 * 256, 256));
    assert_eq!(r.anim.record.unwrap().frames, 8);
    fx.assert_clean();
}

#[test]
fn a_name_not_in_the_file_gets_the_default_record() {
    let mut fx = Fx::new();
    with_anim(&mut fx);
    fx.sim
        .hooks()
        .x
        .names
        .insert((UnitType::Player, 7), *b"SOA1HTH\0");
    let p = fx.spawn(UnitType::Player, 1, fx.a, 10, 10);
    // A loaded player's attack rate (stat 68 = 100), which the schedule
    // reads in mode 7.
    fx.stats(p, &[(68, 100)]);
    animate(&mut fx, p, 7);
    // Frames 2048, speed 256, no events → only the end. The Sorceress's
    // A1 without a weapon has frame bonus 1 (type class 0), so the
    // schedule starts at index 1 and ends at f + 2047.
    let r = fx.sim.sys.units.get(p).unwrap();
    assert_eq!(r.anim.record.unwrap().frames, 2048);
    assert_eq!(fx.timers(p), [(event::END_ANIM, 2047)]);
    fx.assert_clean();
}

#[test]
fn without_a_composed_name_there_is_no_record() {
    let mut fx = Fx::new();
    with_anim(&mut fx);
    let p = fx.spawn(UnitType::Player, 1, fx.a, 10, 10);
    let s = &mut fx.sim.sys;
    let mut sim = Sim {
        game: &mut fx.game,
        units: &mut s.units,
        stats: &mut s.stats,
        data: &s.data,
    };
    modes::set_mode(&mut sim, &mut s.hooks, p, 8).unwrap();
    assert!(modes::animate(&mut sim, &mut s.hooks, p).is_err());
}

/// experience.txt: max level 3; thresholds 0, 500, 1500.
fn vitals() -> VitalsTables {
    let row = |v: u32| Experience {
        amazon: v,
        sorceress: v,
        necromancer: v,
        paladin: v,
        barbarian: v,
        druid: v,
        assassin: v,
        ..blank()
    };
    VitalsTables {
        charstats: Vec::new(),
        experience: vec![row(3), row(0), row(500), row(1500)],
    }
}

/// A player owner at (10, 10) (AR 300, level 1) and a monster at (13,
/// 10) (10 life, level 1, 100 experience), the monster's target flags
/// and collision bit set.
fn kill_setup(fx: &mut Fx) -> (UnitId, UnitId) {
    with_anim(fx);
    fx.sim.hooks().vitals = Some(Arc::new(vitals()));
    let p = fx.spawn(UnitType::Player, 1, fx.a, 10, 10);
    let m = fx.spawn(UnitType::Monster, 0, fx.a, 13, 10);
    fx.stats(p, &[(TOHIT, 300), (LEVEL_STAT, 1)]);
    fx.stats(
        m,
        &[
            (LEVEL_STAT, 1),
            (EXPERIENCE, 100),
            (st::MAXHP, 2560),
            (st::HITPOINTS, 2560),
        ],
    );
    fx.sim.sys.units.get_mut(m).unwrap().flags |=
        unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED;
    fx.mark(m, bits::MONSTER);
    (p, m)
}

fn fire(fx: &mut Fx, owner: UnitId, tx: i32) -> UnitId {
    let p = MissileParams {
        owner: Some(owner),
        origin: Some(owner),
        class: 0,
        flags: param_flags::TARGET_ABSOLUTE,
        target_x: tx,
        target_y: 10,
        ..MissileParams::default()
    };
    let m = fx
        .sim
        .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &p))
        .unwrap()
        .expect("created");
    // The missile's damage setup is not part of this wiring: set here.
    fx.stats(m, &[(MINDAMAGE, 2560), (MAXDAMAGE, 2560)]);
    m
}

/// A kill after a missile hit: life 0 gives result 2, a monster that will
/// die is killed, the kill steps run in order with the death mode change
/// taking the attacker as target, and the experience uses level factor 256
/// for equal levels: 100 (no level-up below 500).
#[test]
fn a_killing_missile_runs_the_kill_and_gives_experience() {
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.seed(p, seed_giving(10));
    fire(&mut fx, p, 13);
    for _ in 0..3 {
        fx.frame();
    }
    assert_eq!(fx.stat(m, st::HITPOINTS), 0);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::DT);
    let (pi, mi) = (p.0, m.0);
    assert_eq!(
        fx.sim.hooks().x.log,
        [
            format!("event 0 Some({mi})"),
            format!("event 11 Some({mi})"),
            format!("event 2 Some({mi})"),
            format!("event 10 Some({mi})"),
            format!("event 9 Some({pi})"),
            format!("reaction {pi} {mi} 0x3"),
            format!("kill PetCredit {mi} {pi}"),
            format!("kill AttackerBookkeeping {mi} {pi}"),
            format!("kill FaceAttacker {mi} {pi}"),
            format!("death start {mi} Some({pi})"),
            format!("kill QuestKill {mi} {pi}"),
            format!("kill BarricadeDoors {mi} {pi}"),
        ]
    );
    // The death animation: 4 frames → end at f + 4.
    let f = fx.game.frame;
    assert_eq!(fx.timers(m), [(event::END_ANIM, f + 4)]);
    assert_eq!(fx.sim.hooks().mode_target, None);
    assert_eq!(fx.stat(p, EXPERIENCE), 100);
    fx.assert_clean();
}

/// The death clean-up cancels the monster's pending think (type 2) and
/// regeneration (type 3) events, so a think scheduled before the kill
/// never runs on the dead unit (the playthrough's "killed monsters stand
/// back up").
#[test]
fn the_death_start_cancels_the_pending_think_and_regeneration() {
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.seed(p, seed_giving(10));
    let f0 = fx.game.frame;
    for ev in [event::AI_THINK, event::STAT_REGEN] {
        fx.game
            .schedule_event(m, u32::from(ev), f0 + 15, None, 0, 0)
            .unwrap();
    }
    fire(&mut fx, p, 13);
    for _ in 0..3 {
        fx.frame();
    }
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::DT);
    let f = fx.game.frame;
    assert_eq!(fx.timers(m), [(event::END_ANIM, f + 4)]);
}

/// The kill of a monster with a think pending at f + 15: no think runs
/// (without the AI store a think would log a re-entrance error), the death
/// ends in mode 12 and the monster is still in mode 12 a hundred frames
/// later. The clean-up's fields: the overhead freed (flag 0x100), flags
/// 0x800C cleared, the target-node index set to 11 and the node gone from
/// its target-node list; the room's dead-GUID ring holds the monster.
#[test]
fn a_killed_monster_with_a_pending_think_stays_dead() {
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.seed(p, seed_giving(10));
    {
        let r = fx.sim.sys.units.get_mut(m).unwrap();
        r.hover = Some(1000);
        r.flags |= 0x8000;
        r.node_index = 8;
    }
    fx.game.target_nodes.push_front(8, m);
    let f0 = fx.game.frame;
    fx.game
        .schedule_event(m, u32::from(event::AI_THINK), f0 + 15, None, 0, 0)
        .unwrap();
    fire(&mut fx, p, 13);
    for _ in 0..3 {
        fx.frame();
    }
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::DT);
    let r = fx.sim.sys.units.get(m).unwrap();
    assert_eq!(r.hover, None);
    assert_ne!(r.flags & crate::units::record::flags::HOVER_FREED, 0);
    assert_eq!(r.flags & 0x800C, 0);
    assert_eq!(r.node_index, 11);
    assert!(fx.game.target_nodes.slot(8).is_empty());
    let guid = fx.game.lists.unit(m).unwrap().guid;
    let room = fx.game.lists.room(fx.a).unwrap();
    assert_eq!((room.dead_guids[0], room.dead_next), (guid, 1));
    for _ in 0..100 {
        fx.frame();
    }
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::DD);
    assert_eq!(fx.stat(m, st::HITPOINTS), 0);
    assert!(fx.timers(m).is_empty());
    fx.assert_clean();
}

/// A monster killed while frozen: the stat-list death frees the freeze
/// list (state 1 is not `monstaydeath`), the keep mask clears the state
/// bit, and the dead monster stays in mode 12 past the frame the freeze
/// would have ended.
#[test]
fn a_monster_killed_while_frozen_stays_dead() {
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.seed(p, seed_giving(10));
    use crate::combat::CombatWorld;
    let freeze = crate::stats::states::state::FREEZE as u16;
    let f0 = fx.game.frame;
    fx.sim.combat(&mut fx.game, |w, _| {
        w.create_state_list(m, freeze, p, f0 + 20);
        w.set_state(m, freeze, true);
        w.schedule_timer(m, event::REMOVE_STATE, f0 + 20);
    });
    fire(&mut fx, p, 13);
    for _ in 0..3 {
        fx.frame();
    }
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::DT);
    assert!(!fx.sim.sys.stats.has_state(m, u32::from(freeze)));
    fx.sim.combat(&mut fx.game, |w, _| {
        assert_eq!(w.state_list_expiry(m, freeze), None);
    });
    for _ in 0..100 {
        fx.frame();
    }
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::DD);
    fx.assert_clean();
}

/// The death keep mask: a non-boss monster keeps its `monstaydeath` states
/// (60 in the fixture's states table) and loses the others,
/// `bossstaydeath` (61) included; the cleared bit is marked changed.
#[test]
fn the_death_keeps_only_the_stay_on_death_states() {
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.seed(p, seed_giving(10));
    for s in [60u32, 61] {
        fx.sim.sys.stats.toggle_state(m, s, true);
    }
    fx.sim.sys.stats.clear_states_changed(m);
    fire(&mut fx, p, 13);
    for _ in 0..3 {
        fx.frame();
    }
    let stats = &fx.sim.sys.stats;
    assert!(stats.has_state(m, 60));
    assert!(!stats.has_state(m, 61));
    let (_, changed) = stats.state_bits(m).unwrap();
    assert_ne!(changed[1] & 1 << (61 - 32), 0);
    assert_eq!(changed[1] & 1 << (60 - 32), 0);
    fx.assert_clean();
}

/// The mode set has no dead guard: a mode-1 request on a dead monster runs
/// the neutral start, mode 1 with hp 0 and a think scheduled. A request
/// for mode 12 on a live monster runs the DD start: the clean-up (its
/// think cancelled), then mode 12.
#[test]
fn mode_requests_on_a_dead_monster_follow_the_starts() {
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.seed(p, seed_giving(10));
    fire(&mut fx, p, 13);
    for _ in 0..10 {
        fx.frame();
    }
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::DD);
    let request = |fx: &mut Fx, u: UnitId, mode: u32| {
        let s = &mut fx.sim.sys;
        let mut sim = Sim {
            game: &mut fx.game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        modes::monster_set_mode(&mut sim, &mut s.hooks, u, mode).unwrap();
    };
    request(&mut fx, m, monster_mode::NU);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::NU);
    assert_eq!(fx.stat(m, st::HITPOINTS), 0);
    assert_eq!(fx.timers(m).first().map(|t| t.0), Some(event::AI_THINK));

    let live = fx.spawn(UnitType::Monster, 0, fx.a, 14, 12);
    let f = fx.game.frame;
    fx.game
        .schedule_event(live, u32::from(event::AI_THINK), f + 15, None, 0, 0)
        .unwrap();
    request(&mut fx, live, monster_mode::DD);
    assert_eq!(fx.sim.sys.units.get(live).unwrap().mode, monster_mode::DD);
    assert!(fx.timers(live).is_empty());
    fx.assert_clean();
}

/// The kill (flag 1) queues the killed monster for the hireling host
/// (`ActionHooks::pet_deaths`), before the pet credit seam; a guarded kill
/// queues nothing; without the queue nothing is recorded.
#[test]
fn the_kill_queues_the_defender_for_the_hireling_host() {
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::kill(w, m, p);
    });
    assert_eq!(fx.sim.hooks().pet_deaths, None);

    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.sim.hooks().pet_deaths = Some(Vec::new());
    fx.sim.sys.units.get_mut(m).unwrap().mode = monster_mode::DD;
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::kill(w, m, p);
    });
    assert_eq!(fx.sim.hooks().pet_deaths, Some(vec![]));
    fx.sim.sys.units.get_mut(m).unwrap().mode = 1;
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::kill(w, m, p);
    });
    assert_eq!(fx.sim.hooks().pet_deaths, Some(vec![m]));
}

/// A dead monster is not killed again, and an uninterruptible defender
/// only gets `death_delay`.
#[test]
fn kill_guards() {
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.sim.sys.units.get_mut(m).unwrap().mode = monster_mode::DD;
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::kill(w, m, p);
    });
    assert!(fx.sim.hooks().x.log.is_empty());
    assert_eq!(fx.stat(p, EXPERIENCE), 0);

    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.sim.with(&mut fx.game, |_, v| {
        v.set_state(m, crate::stats::states::state::UNINTERRUPTABLE as u16, true)
    });
    let mut rec = crate::combat::DamageRecord {
        result: 3,
        ..Default::default()
    };
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::reaction(w, p, m, &mut rec);
    });
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 1);
    let has_92 = fx.sim.sys.stats.has_state(m, 92);
    assert!(has_92);
    assert_eq!(
        fx.sim.hooks().x.log,
        [format!("reaction {} {} 0x3", p.0, m.0)]
    );
}

/// A victim with unit flag 0x04000000 gives no experience, the rest of the
/// kill runs; a player victim in mode 0 / 17 stops at the guard, a live
/// one runs the first two steps only (no death mode request, no quest
/// parse).
#[test]
fn kill_without_experience_and_player_victims() {
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.sim.sys.units.get_mut(m).unwrap().flags |=
        crate::wiring::action::reaction::UNIT_FLAG_NO_EXPERIENCE;
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::kill(w, m, p);
    });
    assert_eq!(fx.stat(p, EXPERIENCE), 0);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::DT);
    // A player victim: dead (17) → nothing; alive → no death mode here.
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    let q = fx.spawn(UnitType::Player, 1, fx.a, 11, 10);
    fx.sim.sys.units.get_mut(q).unwrap().mode = 17;
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::kill(w, q, m);
    });
    assert!(fx.sim.hooks().x.log.is_empty());
    fx.sim.sys.units.get_mut(q).unwrap().mode = 1;
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::kill(w, q, m);
    });
    let (qi, mi) = (q.0, m.0);
    assert_eq!(
        fx.sim.hooks().x.log,
        [format!("kill AttackerBookkeeping {qi} {mi}")]
    );
    assert_eq!(fx.sim.sys.units.get(q).unwrap().mode, 1);
    let _ = p;
}

// ---- the drop ---------------------------------------------------------------------------

/// Gold only (type 4 under misc), TC 1 = one pick of gold.
pub(super) fn drop_tables() -> DropTables {
    let n: usize = 40;
    let words = n.div_ceil(32);
    let mut equiv = EquivMatrix {
        n,
        words,
        bits: vec![0; n * words],
    };
    for i in 1..n {
        equiv.bits[i * words] |= 1;
        equiv.bits[i * words + i / 32] |= 1 << (i % 32);
    }
    let (g, misc) = (usize::from(ty::GOLD), usize::from(ty::MISC));
    equiv.bits[g * words + misc / 32] |= 1 << (misc % 32);
    let mut itemtypes: Vec<Itemtypes> = (0..n)
        .map(|_| {
            let mut t: Itemtypes = blank();
            (t.class, t.staffmods, t.rare) = (0xFF, 0xFF, 1);
            t
        })
        .collect();
    // Gold is always normal quality (itemtypes `Normal`).
    itemtypes[g].normal = 1;
    let mut ratio: Itemratio = blank();
    ratio.version = 1;
    let items = ItemTables {
        items: vec![ItemRec {
            code: *b"gld ",
            type_: ty::GOLD as i16,
            level: 1,
            ..ItemRec::default()
        }],
        itemtypes,
        equiv,
        itemratio: vec![ratio],
        valshift: vec![0; 359],
        stat_shift: 6,
        stat_mask: 0x3F,
        ..ItemTables::default()
    };
    let treasure_items = vec![ItemData {
        code: *b"gld ",
        ubercode: [0; 4],
        ultracode: [0; 4],
        version: 0,
        level: 1,
        type_: ty::GOLD,
        type2: 0,
        unique: 0,
        quest: 0,
        spawnable: 1,
    }];
    let tc = |name: &[u8], entries: Vec<TcEntry>, total| TreasureClass {
        name: name.to_vec(),
        group: 0,
        level: 0,
        total_classic: total,
        total_expansion: total,
        picks: 1,
        nodrop: 0,
        mods: [0; 6],
        entries,
    };
    let gold = TcEntry {
        start_classic: 0,
        start_expansion: 0,
        id: 0,
        row: 0,
        flags: 0,
        mods: [0; 6],
    };
    DropTables {
        items,
        tcs: TreasureClasses {
            tcs: vec![tc(b"none", Vec::new(), 0), tc(b"gold", vec![gold], 1)],
            group_offset: 0,
            chest: [None; 45],
            notes: Vec::new(),
        },
        treasure_items,
        superuniques: Vec::new(),
    }
}

/// The start spot as is.
struct Here;

impl FreeSpot for Here {
    fn free_spot(
        &mut self,
        room: Option<RoomId>,
        start: (i32, i32),
        _: (i32, i32),
    ) -> Option<DropSpot> {
        Some(DropSpot {
            room,
            x: start.0,
            y: start.1,
        })
    }
}

/// A monster of class 0 with treasure class 1 in a game with the drop
/// state; returns the drop state and the monster.
fn drop_setup(fx: &mut Fx) -> (DeathDrops, UnitId, UnitId) {
    let mut t = (*fx.sim.hooks().tables).clone();
    let mut m: Monstats = t.combat.monstats[0].clone();
    m.treasureclass1 = 1;
    t.combat.monstats[0] = m;
    fx.sim.hooks().tables = Arc::new(t);
    let p = fx.spawn(UnitType::Player, 1, fx.a, 10, 10);
    let mon = fx.spawn(UnitType::Monster, 0, fx.a, 13, 10);
    fx.stats(mon, &[(LEVEL_STAT, 3)]);
    let d = DeathDrops::new(
        Arc::new(drop_tables()),
        GameFields::new(Seed::init(), false),
    );
    (d, p, mon)
}

fn run_drop(fx: &mut Fx, d: &mut DeathDrops, mon: UnitId, p: UnitId) -> Vec<UnitId> {
    let s = &mut fx.sim.sys;
    let mut sim = Sim {
        game: &mut fx.game,
        units: &mut s.units,
        stats: &mut s.stats,
        data: &s.data,
    };
    monster_death_drop(&mut s.hooks, &mut sim, d, &mut Here, mon, Some(p))
}

/// The walk on the monster's seed picks
/// the gold (one draw: `roll(1)`), the item is created on the game seed
/// (allocation and item seeds: two steps) at the start spot (x + 2,
/// y + 3, in the same room) and added to the room's units, in mode 3.
#[test]
fn a_dead_monster_drops_a_real_item_into_its_room() {
    let mut fx = Fx::new();
    let (mut d, p, mon) = drop_setup(&mut fx);
    let mut want_mon = fx.sim.sys.units.get(mon).unwrap().seed;
    want_mon.roll(1);
    let mut want_game = fx.sim.hooks().game_seed;
    want_game.step();
    want_game.step();
    let out = run_drop(&mut fx, &mut d, mon, p);
    assert!(
        d.failures.is_empty() && d.errors.is_empty(),
        "{:?} {:?}",
        d.failures,
        d.errors
    );
    assert_eq!(out.len(), 1);
    let item = out[0];
    let a = fx.a;
    assert_eq!(
        d.placed,
        [(
            item,
            DropSpot {
                room: Some(a),
                x: 15,
                y: 13
            }
        )]
    );
    assert_eq!(fx.game.lists.unit(item).unwrap().room(), Some(a));
    assert!(fx.game.lists.units_of_type(UnitType::Item).contains(&item));
    let r = fx.sim.sys.units.get(item).unwrap();
    assert_eq!((r.ty, r.class, r.mode), (UnitType::Item, 0, 3));
    assert_eq!(fx.sim.sys.hooks.items.get(item).unwrap().ilvl, 3);
    assert!(fx.stat(item, GOLD_STAT) > 0);
    assert_eq!(fx.sim.sys.units.get(mon).unwrap().seed, want_mon);
    assert_eq!(fx.sim.hooks().game_seed, want_game);
    assert_eq!(d.fields.seed, want_game);
    fx.assert_clean();
}

/// Unit flag 0x20000 or a wall / door bit at the position → no
/// drop, no draw.
#[test]
fn the_gate_stops_the_drop() {
    for case in 0..2 {
        let mut fx = Fx::new();
        let (mut d, p, mon) = drop_setup(&mut fx);
        if case == 0 {
            fx.sim.sys.units.get_mut(mon).unwrap().flags |= 0x20000;
        } else {
            fx.mark(mon, bits::DOOR);
        }
        let seed = fx.sim.sys.units.get(mon).unwrap().seed;
        assert!(run_drop(&mut fx, &mut d, mon, p).is_empty());
        assert!(d.placed.is_empty());
        assert_eq!(fx.sim.sys.units.get(mon).unwrap().seed, seed);
        assert!(fx.game.lists.units_of_type(UnitType::Item).is_empty());
    }
}

/// Find Item has no gate, so a corpse whose gate
/// refuses (unit flag 0x20000) still drops; the walk draws on the
/// corpse's unit seed (`roll(1)` of the gold pick) with the caster as R,
/// and the item is created and placed as a death drop's.
#[test]
fn find_item_drops_from_the_corpse_without_the_gate() {
    let mut fx = Fx::new();
    let (mut d, p, mon) = drop_setup(&mut fx);
    fx.sim.sys.units.get_mut(mon).unwrap().flags |= 0x20000;
    let mut want_mon = fx.sim.sys.units.get(mon).unwrap().seed;
    want_mon.roll(1);
    let out = {
        let s = &mut fx.sim.sys;
        let mut sim = Sim {
            game: &mut fx.game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        crate::wiring::economy::find_item_drop(&mut s.hooks, &mut sim, &mut d, &mut Here, mon, p)
    };
    assert!(d.failures.is_empty() && d.errors.is_empty());
    assert_eq!(out.len(), 1);
    assert_eq!(d.placed.len(), 1);
    assert!(fx.stat(out[0], GOLD_STAT) > 0);
    assert_eq!(fx.sim.sys.units.get(mon).unwrap().seed, want_mon);
    fx.assert_clean();
}

/// A monster at the east edge of room A whose start (x + 2, y + 3) lies in
/// room B: the free-spot search gets the monster's room A, never the start
/// lookup's B; the start-spot fill ([`crate::wiring::economy::StartSpot`])
/// keeps the start in B.
#[test]
fn the_free_spot_search_gets_the_monsters_room() {
    let mut fx = Fx::new();
    let (mut d, p, _) = drop_setup(&mut fx);
    let mon = fx.spawn(UnitType::Monster, 0, fx.a, 38, 10);
    fx.stats(mon, &[(LEVEL_STAT, 3)]);
    let (a, b) = (fx.a, fx.b);
    let run = |fx: &mut Fx, d: &mut DeathDrops, mon: UnitId, start_spot: bool| {
        let s = &mut fx.sim.sys;
        let mut sim = Sim {
            game: &mut fx.game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        if start_spot {
            monster_death_drop(
                &mut s.hooks,
                &mut sim,
                d,
                &mut crate::wiring::economy::StartSpot,
                mon,
                Some(p),
            )
        } else {
            monster_death_drop(&mut s.hooks, &mut sim, d, &mut Here, mon, Some(p))
        }
    };
    assert_eq!(run(&mut fx, &mut d, mon, false).len(), 1);
    assert_eq!(
        d.placed[0].1,
        DropSpot {
            room: Some(a),
            x: 40,
            y: 13
        }
    );
    let mon2 = fx.spawn(UnitType::Monster, 0, fx.a, 38, 10);
    fx.stats(mon2, &[(LEVEL_STAT, 3)]);
    assert_eq!(run(&mut fx, &mut d, mon2, true).len(), 1);
    assert_eq!(
        d.placed[1].1,
        DropSpot {
            room: Some(b),
            x: 40,
            y: 13
        }
    );
}

/// The player count counts the players not dead (mode 0 or 17, flag
/// 0x10000); the `players` command ignores values above 8.
#[test]
fn living_players_and_the_players_setting() {
    let mut fx = Fx::new();
    let (mut d, p, _) = drop_setup(&mut fx);
    let q = fx.spawn(UnitType::Player, 1, fx.a, 12, 12);
    let count = |fx: &mut Fx| {
        let s = &mut fx.sim.sys;
        let sim = Sim {
            game: &mut fx.game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        crate::wiring::economy::death::living_players(&sim)
    };
    for (u, mode) in [(p, 1), (q, 1)] {
        fx.sim.sys.units.get_mut(u).unwrap().mode = mode;
    }
    assert_eq!(count(&mut fx), 2);
    fx.sim.sys.units.get_mut(q).unwrap().mode = 17;
    assert_eq!(count(&mut fx), 1);
    d.set_players(8);
    assert_eq!(d.players_setting, 8);
    d.set_players(9);
    assert_eq!(d.players_setting, 8);
}

/// The drop fixture with the path provider on and the synthetic
/// walk-back field (`path::search` tests: vectors F1–F3) loaded.
fn drop_setup_paths(fx: &mut Fx) -> (DeathDrops, UnitId, UnitId) {
    let h = fx.sim.hooks();
    h.enable_paths().expect("embedded tables");
    h.paths.as_mut().unwrap().field = Some(Arc::new(crate::path::search::tests::sign_field()));
    drop_setup(fx)
}

/// Sets collision bits on the cell (x, y) of room A's grid.
fn set_cell(fx: &mut Fx, x: i32, y: i32, bit: u16) {
    let (a, game) = (fx.a, &fx.game);
    *fx.sim
        .sys
        .hooks
        .drlg
        .collision_mut(game, a, x, y)
        .expect("in a grid") |= bit;
}

fn spot_of(fx: &mut Fx, d: &mut DeathDrops, mon: UnitId, p: UnitId) -> DropSpot {
    let before = d.placed.len();
    let out = run_drop(fx, d, mon, p);
    assert_eq!(out.len(), 1, "{:?} {:?}", d.failures, d.errors);
    assert_eq!(d.placed.len(), before + 1);
    d.placed[before].1
}

/// The floor drop with the path provider (vector D2 translated by
/// (+3, 0)): the monster at (13, 10) by its path record (the `Pending`
/// position is not set with the provider on), an empty room → the start
/// (15, 13). The item gets its static path at the spot and its footprint
/// (0x200).
#[test]
fn with_the_path_provider_the_drop_lands_on_the_floor_drop_spot() {
    let mut fx = Fx::new();
    let (mut d, p, mon) = drop_setup_paths(&mut fx);
    assert_eq!(fx.sim.hooks().path_position(mon), (13, 10));
    let a = fx.a;
    let spot = spot_of(&mut fx, &mut d, mon, p);
    assert_eq!(
        spot,
        DropSpot {
            room: Some(a),
            x: 15,
            y: 13
        }
    );
    let item = d.placed[0].0;
    assert!(matches!(
        fx.sim.hooks().paths.as_ref().unwrap().record(item),
        Some(crate::path::UnitPath::Static(_))
    ));
    assert_eq!(fx.sim.hooks().path_position(item), (15, 13));
    let cell = |fx: &mut Fx, x, y| {
        crate::path::collision::point_value(&fx.sim.hooks().drlg, Some(a), x, y, 0xFFFF)
    };
    assert_ne!(cell(&mut fx, 15, 13) & bits::ITEM, 0);
    fx.assert_clean();
}

/// Vector D1 translated: a wall column at x = 15 (the start's column)
/// → ring 1: (14, 12) d 2 kept, (16, *) fail the walk-back through the
/// wall, (14, 13) d 1 wins. Vector D3 translated: only an item bit
/// (0x200) at the start → (14, 13). The same drop without the blocked
/// cells lands on the start (previous test).
#[test]
fn the_floor_drop_avoids_blocked_cells() {
    for case in 0..2 {
        let mut fx = Fx::new();
        let (mut d, p, mon) = drop_setup_paths(&mut fx);
        if case == 0 {
            for y in 0..40 {
                set_cell(&mut fx, 15, y, bits::WALL);
            }
        } else {
            set_cell(&mut fx, 15, 13, bits::ITEM);
        }
        let a = fx.a;
        assert_eq!(
            spot_of(&mut fx, &mut d, mon, p),
            DropSpot {
                room: Some(a),
                x: 14,
                y: 13
            },
            "case {case}"
        );
        fx.assert_clean();
    }
}

/// Each drop sees the previous ones: the first drop's item footprint
/// (0x200, in mask 0x3E01) blocks the start for the next, which lands as
/// in vector D3 (14, 13).
#[test]
fn a_dropped_item_blocks_the_next_drop() {
    let mut fx = Fx::new();
    let (mut d, p, mon) = drop_setup_paths(&mut fx);
    let a = fx.a;
    let first = spot_of(&mut fx, &mut d, mon, p);
    let second = spot_of(&mut fx, &mut d, mon, p);
    assert_eq!((first.x, first.y), (15, 13));
    assert_eq!(
        second,
        DropSpot {
            room: Some(a),
            x: 14,
            y: 13
        }
    );
    fx.assert_clean();
}

/// The collision word with the path provider: a monster at
/// a cell outside every room (no room among its room and the adjacent
/// rooms) reads 0x27, non-zero → no drop, no draw. This is one part of
/// the gate.
#[test]
fn a_monster_outside_every_room_grid_drops_nothing() {
    let mut fx = Fx::new();
    let (mut d, p, _mon) = drop_setup_paths(&mut fx);
    let far = fx.spawn(UnitType::Monster, 0, fx.a, 5000, 5000);
    fx.stats(far, &[(LEVEL_STAT, 3)]);
    assert_eq!(fx.sim.hooks().path_position(far), (5000, 5000));
    let seed = fx.sim.sys.units.get(far).unwrap().seed;
    assert!(run_drop(&mut fx, &mut d, far, p).is_empty());
    assert!(d.placed.is_empty());
    assert_eq!(fx.sim.sys.units.get(far).unwrap().seed, seed);
}

/// The provider on without the walk-back field: the drop keeps the
/// [`FreeSpot`] seam (here: the start as is), and the item is put there
/// (its static path at the spot).
#[test]
fn without_the_field_the_drop_keeps_the_free_spot_seam() {
    let mut fx = Fx::new();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let (mut d, p, mon) = drop_setup(&mut fx);
    // A wall at the start: the seam answers the start regardless.
    set_cell(&mut fx, 15, 13, bits::WALL);
    let spot = spot_of(&mut fx, &mut d, mon, p);
    assert_eq!((spot.x, spot.y), (15, 13));
    let item = d.placed[0].0;
    assert_eq!(fx.sim.hooks().path_position(item), (15, 13));
    fx.assert_clean();
}

fn run_quest_drop(
    fx: &mut Fx,
    d: &mut DeathDrops,
    unit: UnitId,
    code: [u8; 4],
    quality: u8,
) -> Option<UnitId> {
    let s = &mut fx.sim.sys;
    let mut sim = Sim {
        game: &mut fx.game,
        units: &mut s.units,
        stats: &mut s.stats,
        data: &s.data,
    };
    crate::wiring::economy::unit_quest_drop(
        &mut s.hooks,
        &mut sim,
        d,
        &mut Here,
        unit,
        Some(code),
        quality,
        -1,
        0,
    )
}

/// The action wiring's drop: the drop code names the class
/// (no draw on the unit seed); item level = the monster's level stat; the
/// request (spawn mode 3, init flags 1, quality) creates a real item at
/// the floor-drop start spot, on two game-seed steps.
#[test]
fn the_quest_drop_creates_the_drop_code_item_at_the_unit() {
    let mut fx = Fx::new();
    let (mut d, _p, mon) = drop_setup(&mut fx);
    let want_mon = fx.sim.sys.units.get(mon).unwrap().seed;
    let mut want_game = fx.sim.hooks().game_seed;
    want_game.step();
    want_game.step();
    let item = run_quest_drop(&mut fx, &mut d, mon, *b"gld ", 2).expect("item");
    assert!(d.failures.is_empty() && d.errors.is_empty());
    let a = fx.a;
    assert_eq!(
        d.placed,
        [(
            item,
            DropSpot {
                room: Some(a),
                x: 15,
                y: 13
            }
        )]
    );
    let r = fx.sim.sys.units.get(item).unwrap();
    assert_eq!((r.ty, r.class, r.mode), (UnitType::Item, 0, 3));
    assert_eq!(fx.sim.sys.hooks.items.get(item).unwrap().ilvl, 3);
    assert_eq!(fx.sim.sys.units.get(mon).unwrap().seed, want_mon);
    assert_eq!(fx.sim.hooks().game_seed, want_game);
    fx.assert_clean();
}

/// A drop code missing from the items table is fatal 0x9EA
/// (logged; no item, no draw).
#[test]
fn the_quest_drop_with_an_unknown_code_is_fatal() {
    let mut fx = Fx::new();
    let (mut d, _p, mon) = drop_setup(&mut fx);
    let game = fx.sim.hooks().game_seed;
    assert_eq!(run_quest_drop(&mut fx, &mut d, mon, *b"zzz ", 2), None);
    // `drop_helpers::source_drop` records its pick
    // fatals in `pick_errors`.
    assert_eq!(
        d.pick_errors,
        [crate::treasure::class_pick::PickError::Code(
            u32::from_le_bytes(*b"zzz ")
        )]
    );
    assert!(d.errors.is_empty());
    assert!(d.placed.is_empty());
    assert_eq!(fx.sim.hooks().game_seed, game);
}

/// A get-hit result the get-hit test lets
/// through puts the monster into get-hit (mode 3, toward the attacker);
/// a soft result only sets unit flag 0x8000 and queues the unit. A dead
/// monster is left alone.
#[test]
fn a_hit_puts_the_monster_into_get_hit_or_marks_it_soft() {
    use crate::combat::result;
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    let mut rec = crate::combat::DamageRecord {
        result: result::HIT | result::GET_HIT,
        total: 2000,
        ..Default::default()
    };
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::reaction(w, p, m, &mut rec);
    });
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::GH);
    assert_eq!(fx.sim.hooks().mode_target, None);

    // Too small a hit: no get-hit, the soft path.
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    let mut rec = crate::combat::DamageRecord {
        result: result::HIT | result::GET_HIT,
        total: 100,
        ..Default::default()
    };
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::reaction(w, p, m, &mut rec);
    });
    let r = fx.sim.sys.units.get(m).unwrap();
    assert_eq!(r.mode, 1);
    assert_ne!(r.flags & 0x8000, 0, "soft hit");

    // A soft result alone; a dead monster is unchanged.
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    let mut rec = crate::combat::DamageRecord {
        result: result::HIT | result::SOFT_HIT,
        total: 100,
        ..Default::default()
    };
    fx.sim.sys.units.get_mut(m).unwrap().mode = monster_mode::DD;
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::reaction(w, p, m, &mut rec);
    });
    assert_eq!(fx.sim.sys.units.get(m).unwrap().flags & 0x8000, 0);
    fx.sim.sys.units.get_mut(m).unwrap().mode = 1;
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::reaction(w, p, m, &mut rec);
    });
    assert_ne!(fx.sim.sys.units.get(m).unwrap().flags & 0x8000, 0);
}

/// A hit stores its class in the damage record; an
/// unfixed class without an element nibble first takes the element hit
/// class (base 3, no element → 3; base 0 → 13 `over`; a cold hit →
/// 0x30 with the counter at 0). A miss stores nothing. The player's soft
/// hit (unit flag 0x8000) reaches its client as S→C 0x0D (type 0, GUID,
/// 0x13, the cell, the hit class, life percent) in the next client pass,
/// once.
#[test]
fn a_soft_hit_on_a_player_stores_the_hit_class_and_sends_0x0d() {
    use crate::combat::result;
    use crate::units::lists::client_state;
    let mut fx = Fx::new();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let (p, m) = kill_setup(&mut fx);
    fx.stats(p, &[(st::MAXHP, 100 << 8), (st::HITPOINTS, 96 << 8)]);
    fx.game
        .lists
        .add_client(Some(p), Some(fx.a), client_state::IN_GAME);
    let hit = |fx: &mut Fx, result: u16, class: u32, cold: i32| {
        let mut rec = crate::combat::DamageRecord {
            result,
            total: 100,
            hit_class: class,
            cold,
            ..Default::default()
        };
        fx.sim.combat(&mut fx.game, |w, _| {
            crate::wiring::action::reaction::reaction(w, m, p, &mut rec);
        });
        fx.sim.sys.units.get(p).unwrap().hit_class
    };
    fx.sim.hooks().hit_class = 0;
    assert_eq!(hit(&mut fx, result::HIT, 0, 0), 13);
    assert_eq!(
        hit(&mut fx, result::HIT, 2, 1),
        0x32,
        "counter 1: fire is 0, cold next"
    );
    assert_eq!(hit(&mut fx, 0, 5, 0), 0x32, "a miss stores nothing");
    assert_eq!(hit(&mut fx, result::HIT | result::SOFT_HIT, 3, 0), 3);
    assert_ne!(fx.sim.sys.units.get(p).unwrap().flags & 0x8000, 0);
    let guid = fx.sim.sys.units.get(p).unwrap().guid;
    fx.sim.hooks().x.sent.clear();
    fx.tick();
    let soft: Vec<Vec<u8>> = fx
        .sim
        .hooks()
        .x
        .sent
        .iter()
        .filter(|(_, m)| m[0] == 0x0D && m[6] == 0x13)
        .map(|(_, m)| m.clone())
        .collect();
    assert_eq!(soft.len(), 1, "{soft:02x?}");
    assert_eq!(soft[0][1], 0);
    assert_eq!(soft[0][2..6], guid.to_le_bytes());
    assert_eq!(soft[0][11..], [3, 96]);
    assert_eq!(
        fx.sim.sys.units.get(p).unwrap().flags & 0x8000,
        0,
        "cleaned up"
    );
    fx.sim.hooks().x.sent.clear();
    fx.tick();
    assert!(!fx.sim.hooks().x.sent.iter().any(|(_, m)| m[0] == 0x0D));
}

fn react(fx: &mut Fx, a: UnitId, d: UnitId, result: u16) {
    let mut rec = crate::combat::DamageRecord {
        result,
        total: 2000,
        ..Default::default()
    };
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::reaction(w, a, d, &mut rec);
    });
}

/// A block result puts a monster
/// whose class has BL into mode 6; a class without BL, Diablo, or a soft
/// block does not change mode. A knockback result on a class with KB
/// enters mode 13; without KB it turns into get-hit.
#[test]
fn a_monster_blocks_and_is_knocked_back() {
    use crate::combat::result;
    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    react(&mut fx, p, m, result::HIT | result::BLOCK);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 6, "BL");

    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    react(
        &mut fx,
        p,
        m,
        result::HIT | result::BLOCK | result::SOFT_HIT,
    );
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 1, "soft block");

    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.sim.hooks().x.missing_modes = vec![6];
    react(&mut fx, p, m, result::HIT | result::BLOCK);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 1, "no BL mode");

    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    react(&mut fx, p, m, result::HIT | result::KNOCKBACK);
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, 13, "KB");

    let mut fx = Fx::new();
    let (p, m) = kill_setup(&mut fx);
    fx.sim.hooks().x.missing_modes = vec![13];
    react(&mut fx, p, m, result::HIT | result::KNOCKBACK);
    assert_eq!(
        fx.sim.sys.units.get(m).unwrap().mode,
        monster_mode::GH,
        "KB without the mode → get-hit"
    );
}

/// A player's block result requests BL (mode
/// 9) and stamps stat 95 with the frame, at most once per
/// `fasterblockrate / 8 + 15` frames; a soft block does nothing.
#[test]
fn a_player_blocks_once_per_block_window() {
    use crate::combat::result;
    const LAST_BLOCK: u16 = 95;
    let mut fx = Fx::new();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let (p, m) = kill_setup(&mut fx);
    fx.stats(p, &[(102, 16), (LAST_BLOCK, 0)]);
    // Window: 16 / 8 + 15 = 17 frames.
    fx.game.frame = 17;
    react(&mut fx, m, p, result::HIT | result::BLOCK);
    assert_eq!(
        fx.sim.sys.stats.unit_total(p, LAST_BLOCK, 0),
        0,
        "17 - 0 ≤ 17"
    );
    fx.game.frame = 18;
    react(
        &mut fx,
        m,
        p,
        result::HIT | result::BLOCK | result::SOFT_HIT,
    );
    assert_eq!(fx.sim.sys.stats.unit_total(p, LAST_BLOCK, 0), 0, "soft");
    react(&mut fx, m, p, result::HIT | result::BLOCK);
    assert_eq!(fx.sim.sys.stats.unit_total(p, LAST_BLOCK, 0), 18);
    assert_eq!(fx.sim.sys.units.get(p).unwrap().mode, 9, "BL");
    fx.game.frame = 18 + 17;
    react(&mut fx, m, p, result::HIT | result::WEAPON_BLOCK);
    assert_eq!(
        fx.sim.sys.stats.unit_total(p, LAST_BLOCK, 0),
        18,
        "inside the window"
    );
    fx.game.frame = 18 + 18;
    react(&mut fx, m, p, result::HIT | result::WEAPON_BLOCK);
    assert_eq!(fx.sim.sys.stats.unit_total(p, LAST_BLOCK, 0), 36);
}

/// A knockback result requests KB
/// (mode 19); a get-hit the get-hit test lets through requests GH (mode
/// 4), a small one is soft.
#[test]
fn a_player_is_knocked_back_or_gets_hit() {
    use crate::combat::result;
    let setup = || {
        let mut fx = Fx::new();
        fx.sim.hooks().enable_paths().expect("embedded tables");
        let (p, m) = kill_setup(&mut fx);
        fx.stats(p, &[(st::MAXHP, 2560), (st::HITPOINTS, 2560)]);
        (fx, p, m)
    };
    let (mut fx, p, m) = setup();
    react(&mut fx, m, p, result::HIT | result::KNOCKBACK);
    assert_eq!(fx.sim.sys.units.get(p).unwrap().mode, 19, "KB");

    let (mut fx, p, m) = setup();
    react(&mut fx, m, p, result::HIT | result::GET_HIT);
    assert_eq!(fx.sim.sys.units.get(p).unwrap().mode, 4, "GH");

    let (mut fx, p, m) = setup();
    let mut rec = crate::combat::DamageRecord {
        result: result::HIT | result::GET_HIT,
        total: 100,
        ..Default::default()
    };
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::reaction(w, m, p, &mut rec);
    });
    let r = fx.sim.sys.units.get(p).unwrap();
    assert_eq!(r.mode, 1);
    assert_ne!(r.flags & 0x8000, 0, "soft hit");
}

/// The death start snaps the monster's path direction toward the killer
/// (a quill rat killed by an arrow from (−4, −4) faces 32).
#[test]
fn death_start_faces_the_killer() {
    let mut fx = Fx::new();
    fx.sim.hooks().enable_paths().expect("embedded tables");
    let (_, m) = kill_setup(&mut fx);
    let k = fx.spawn(UnitType::Player, 1, fx.a, 9, 6);
    let dir = |fx: &mut Fx| {
        let p = fx.sim.hooks().paths.as_ref().unwrap().dynamic(m).unwrap();
        (p.direction, p.new_direction)
    };
    assert_eq!(dir(&mut fx), (0, 0));
    fx.sim.combat(&mut fx.game, |w, _| {
        crate::wiring::action::reaction::kill(w, m, k);
    });
    assert_eq!(fx.sim.sys.units.get(m).unwrap().mode, monster_mode::DT);
    assert_eq!(dir(&mut fx), (32, 32));
}

/// A champion (monster data type flag 4) drops from column 2. The flag is
/// read from the monster
/// data in the lent world (a champion Fallen's drop takes 10 draws on its
/// seed and makes three items). Without the flag, column 1 (the empty TC) drops nothing.
#[test]
fn a_champion_drops_from_column_2_by_its_monster_data() {
    let run = |champion: bool| {
        let mut fx = Fx::new();
        let (mut d, p, mon) = drop_setup(&mut fx);
        let mut t = (*fx.sim.hooks().tables).clone();
        t.combat.monstats[0].treasureclass1 = 0;
        t.combat.monstats[0].treasureclass2 = 1;
        fx.sim.hooks().tables = Arc::new(t);
        let data = crate::monsters::init::MonsterData {
            type_flags: if champion { 4 } else { 0 },
            ..Default::default()
        };
        let world = super::sound::DataOnly([(mon, data)].into_iter().collect());
        fx.sim.sys.hooks.monster_world = Some(Box::new(world));
        run_drop(&mut fx, &mut d, mon, p).len()
    };
    assert_eq!(run(true), 1);
    assert_eq!(run(false), 0);
}

/// The frame bonus on the mode start (stored as bonus · 256): A1 without
/// a weapon gives 1 for the Amazon and Sorceress
/// (type class 0), 0 for the Barbarian; mode 1 gives 0. Recorded:
/// `check-combat-melee-fallen-msg` f34, the Amazon's A1 starts at frame
/// 256.
#[test]
fn player_attack_starts_at_the_frame_bonus() {
    for (class, mode, frame) in [(0, 7, 256), (1, 8, 256), (4, 7, 0), (0, 1, 0)] {
        let mut fx = Fx::new();
        with_anim(&mut fx);
        let p = fx.spawn(UnitType::Player, class, fx.a, 10, 10);
        let s = &mut fx.sim.sys;
        let mut sim = Sim {
            game: &mut fx.game,
            units: &mut s.units,
            stats: &mut s.stats,
            data: &s.data,
        };
        modes::set_mode(&mut sim, &mut s.hooks, p, mode).unwrap();
        let f = fx.sim.sys.units.get(p).unwrap().anim.frame;
        assert_eq!(f, frame, "class {class} mode {mode}");
    }
}
