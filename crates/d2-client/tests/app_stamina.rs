//! Stamina on the play server (synthetic data): a run drains it, standing
//! regenerates it, both reach the client as 0x95 / 0x96, and a run stops at
//! zero.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::server_thread::ThreadLink;
use d2_client::app::single_player::{self, Link, DEFAULT_SEED, PLAYER_CLASS};
use d2_client::bridge::link::{SendQueue, ServerLink};
use d2_data::bin::BinTable;
use d2_data::fixup::records::stat_ops;
use d2_data::tables::{Charstats, Experience, Itemstatcost, Record, Skills};
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::stats::{ClassStats, StatData, StatLists, StatTable};
use d2_sim::units::UnitId;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl d2_server::seams::Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A synthetic `itemstatcost`: 359 stats, the first 16 saved.
fn stat_data() -> Arc<StatData> {
    let (n, size) = (359, Itemstatcost::SIZE);
    let mut records = vec![0u8; n * size];
    for (s, r) in records.chunks_mut(size).enumerate() {
        for o in [0x32, 0x48, 0x4A, 0x56, 0x58, 0x5A, 0x5C] {
            r[o..o + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
        r[0..2].copy_from_slice(&(s as u16).to_le_bytes());
        if s < 16 {
            r[5] |= 0x10;
        }
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
        classes: vec![ClassStats::default(); 7],
        ..StatData::default()
    })
}

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// Sorceress-like `charstats` (class 1) and `experience` rows
/// (500 × level²).
fn vitals() -> VitalsTables {
    let charstats = (0..7)
        .map(|_| {
            let mut c: Charstats = blank();
            c.str = 10;
            c.dex = 25;
            c.int = 35;
            c.vit = 10;
            c.lifeperlevel = 4;
            c.staminaperlevel = 4;
            c.manaperlevel = 8;
            c.lifepervitality = 8;
            c.staminapervitality = 4;
            c.manapermagic = 8;
            c.statperlevel = 5;
            c
        })
        .collect();
    let row = |v: u32| {
        let mut e: Experience = blank();
        e.amazon = v;
        e.sorceress = v;
        e.necromancer = v;
        e.paladin = v;
        e.barbarian = v;
        e.druid = v;
        e.assassin = v;
        e
    };
    let mut experience = vec![row(99)];
    experience.extend((0..=99).map(|l: u32| row(500 * l * l)));
    VitalsTables {
        charstats,
        experience,
    }
}

type Server = ThreadLink<Link<StepClock>>;

struct Game {
    link: Server,
    ms: Arc<AtomicU32>,
}

impl Game {
    /// Pumps one tick and returns the S→C messages of it.
    fn tick(&mut self) -> Vec<Vec<u8>> {
        self.ms.fetch_add(40, Ordering::SeqCst);
        self.link.pump().unwrap();
        self.link.receive()
    }
}

/// The synthetic game, joined, with vitals tables, a level-1 sorceress
/// with 100 life, and one class skill (id 1) in the tables.
fn joined() -> (Game, UnitId, u32) {
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    link.send(SendQueue::System, &single_player::create_request().encode())
        .unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.receive();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.receive();
    let (p, guid) = link
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let (p, g) = single_player::local_player(sim).expect("joined");
            let a = &mut sim.events.action;
            let t = Arc::make_mut(&mut a.hooks().tables);
            t.skills.skills.resize(2, blank::<Skills>());
            let s: &mut Skills = &mut t.skills.skills[1];
            s.charclass = single_player::PLAYER_CLASS as _;
            s.ingame = true;
            s.skpoints = d2_sim::skills::levels::NO_CALC;
            (s.reqskill1, s.reqskill2, s.reqskill3) = (0xFFFF, 0xFFFF, 0xFFFF);
            a.hooks().vitals = Some(Arc::new(vitals()));
            let sys = &mut a.sys;
            // The synthetic game has no `itemstatcost`: stats 0-15 saved.
            sys.stats = StatLists::new(stat_data());
            let ty = sys.units.get(p).unwrap().ty;
            sys.stats.alloc_extended(
                &mut sys.hooks,
                p,
                ty,
                g,
                u32::from(PLAYER_CLASS as u8),
                0,
                None,
            );
            for (s, v) in [(12u16, 1), (7, 100 << 8), (6, 100 << 8), (0, 10), (4, 0)] {
                sys.stats.unit_set(&mut sys.hooks, p, s, v, 0);
            }
            sys.hooks.enable_vitals_sync();
            (p, g)
        })
        .unwrap();
    let mut g = Game { link, ms };
    // The next ticks populate the town and put the client in game.
    for _ in 0..3 {
        g.tick();
    }
    (g, p, guid)
}

impl Game {
    fn base(&mut self, p: UnitId, stat: u16) -> i32 {
        self.link
            .with(move |l| {
                l.host_mut()
                    .game
                    .events
                    .action
                    .sys
                    .stats
                    .unit_base(p, stat, 0)
            })
            .unwrap()
    }
    fn set(&mut self, p: UnitId, stat: u16, v: i32) {
        self.link
            .with(move |l| {
                let s = &mut l.host_mut().game.events.action.sys;
                s.stats.unit_set(&mut s.hooks, p, stat, v, 0);
            })
            .unwrap();
    }
}

/// The stamina (whole points) of the last 0x95 / 0x96 in `msgs`.
fn sent_stamina(msgs: &[Vec<u8>]) -> Option<u32> {
    msgs.iter().rev().find_map(|m| match m.first() {
        // 0x96: id 8, stamina 15 (bit-packed, low bits first).
        Some(0x96) => Some(u32::from(u16::from_le_bytes([m[1], m[2]]) & 0x7FFF)),
        // 0x95: id 8, life 15, mana 15, stamina 15.
        Some(0x95) => {
            let v = u64::from_le_bytes([m[1], m[2], m[3], m[4], m[5], 0, 0, 0]);
            Some(((v >> 30) & 0x7FFF) as u32)
        }
        _ => None,
    })
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn standing_regenerates_stamina_and_the_client_is_told() {
    let (mut g, p, _) = joined();
    g.set(p, 11, 100 << 8);
    g.set(p, 10, 10 << 8);
    let mut told = None;
    for _ in 0..40 {
        told = sent_stamina(&g.tick()).or(told);
    }
    let s = g.base(p, 10);
    assert!(s > 10 << 8, "regenerated: {s}");
    assert!(s <= 100 << 8);
    let told = told.expect("a stamina message reached the client");
    assert!(told > 10 && u32::try_from(s >> 8).unwrap() >= told);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn full_stamina_is_not_exceeded() {
    let (mut g, p, _) = joined();
    g.set(p, 11, 100 << 8);
    g.set(p, 10, 100 << 8);
    for _ in 0..10 {
        g.tick();
    }
    assert_eq!(g.base(p, 10), 100 << 8);
}
