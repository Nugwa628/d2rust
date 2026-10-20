//! The combat / missile fixtures of the end-to-end combat path, shared by
//! the client's single-player e2e (`d2-client/tests/e2e_single_player.rs`)
//! and the "fight" tick bench (`benches/sim.rs`) through
//! `d2_sim::bench_fixtures::combat`. Synthetic tables only; no game logic
//! lives here.
//!
//! [`Fight`] is the loaded tick of the bench: rows of a player firing
//! arrows at a monster on the action fixture ([`Fx`]), so a tick runs
//! missile flights, hits, damage, kills (death mode, experience, level
//! up) and the monsters' AI on top of the timer queue.

use d2_data::tables::{
    Charstats, Difficultylevels, Experience, Levels, Missiles as MissileRow, Monstats, Monstats2,
    Skilldesc, Skills,
};
use d2_formats::animdata::{self, AnimData, AnimRecord};

use super::{blank, Fx};
use crate::combat::vitals::VitalsTables;
use crate::combat::CombatTables;
use crate::drlg::collision::bits;
use crate::missiles::{create_missile, param_flags, unit_flag, MissileParams};
use crate::skills::{SkillTables, LEVEL_CAP_114D};
use crate::stats::stat as st;
use crate::units::lists::client_state;
use crate::units::{UnitId, UnitType};
use crate::wiring::action::{ActionTables, Pending};
use std::sync::Arc;

/// The right skill of the e2e cast (the Multiple Shot slot).
pub const MULTI: i32 = 1;

/// The fixture's COF names for the animation composer: the sorceress
/// casting (SC) and the monster dying (DT). Other modes get no name.
pub const PLAYER_SC: &[u8; 8] = b"SOSCHTH\0";
/// COF name of the monster dying (DT); see [`PLAYER_SC`].
pub const MONSTER_DT: &[u8; 8] = b"M0DTHTH\0";

/// Monster class 0: killable, AI 1 (Idle), no skills, no minions; level
/// 1, 5 life, 100 experience (`noRatio`: the monstats values as they
/// are); treasure class 1 (Normal).
pub fn monster_class() -> Monstats {
    let mut m: Monstats = blank();
    m.killable = true;
    m.noratio = true;
    m.level = 1;
    (m.minhp, m.maxhp, m.exp) = (5, 5, 100);
    m.treasureclass1 = 1;
    m.velocity = 1;
    (m.drain, m.drain_n, m.drain_h) = (100, 100, 100);
    m.montype = 0xFFFF;
    m.ai = 1;
    (m.aidel, m.aidel_n, m.aidel_h) = (15, 15, 15);
    (m.skill1, m.skill2, m.skill3) = (0xFFFF, 0xFFFF, 0xFFFF);
    m.rarity = 1;
    (m.mingrp, m.maxgrp) = (1, 1);
    (m.minion1, m.minion2) = (0xFFFF, 0xFFFF);
    m.enabled = true;
    m.isspawn = true;
    m
}

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
        &mut s.delay,
        &mut s.perdelay,
    ] {
        *f = 0xFFFF_FFFF;
    }
    s.skilldesc = 0xFFFF;
    s.charclass = 0xFF;
    (s.reqskill1, s.reqskill2, s.reqskill3) = (0xFFFF, 0xFFFF, 0xFFFF);
    s.itypea1 = 0xFFFF;
    s.srvmissile = 0xFFFF;
    s.intown = true;
    s.ingame = true;
    s
}

/// Missile 0: an arrow-like row (default flight, one sub-tile per frame,
/// collide type 3, kill on collision, to-hit), as the action wiring's
/// tests use.
pub fn arrow() -> MissileRow {
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

/// The right skill: start function 53 (MonInferno start, standing in for
/// Multiple Shot's 4, whose body needs items; every filled start slot has
/// a body: with `calc2` = `lvl` it sets the used entry's param 1 := frame +
/// level, which the e2e fakes log, and returns 1), do function 53
/// (filled, `unreferenced`, no body: the seam, standing in for Multiple
/// Shot's 8), `srvmissile` 0 (the generic missile).
pub fn skills() -> SkillTables {
    let mut v = vec![skill_rec(), skill_rec()];
    let m = &mut v[MULTI as usize];
    (m.srvstfunc, m.mana, m.lvlmana, m.manashift) = (53, 4, 1, 8);
    (m.srvdofunc, m.srvmissile, m.calc2) = (53, 0, 0);
    SkillTables {
        skills: v,
        skilldesc: vec![blank::<Skilldesc>()],
        missiles: vec![arrow()],
        // Formula 0: `lvl`.
        skills_code: vec![0x04, 0x10, 0x00],
        miss_code: Vec::new(),
        level_cap: LEVEL_CAP_114D,
        stat_count: 359,
    }
}

pub fn combat_tables() -> CombatTables {
    let mut d: Difficultylevels = blank();
    (d.monsterfreezedivisor, d.monstercolddivisor) = (1, 1);
    (d.lifestealdivisor, d.manastealdivisor) = (1, 1);
    CombatTables {
        charstats: vec![blank::<Charstats>(); 7],
        difficultylevels: vec![d; 3],
        monstats: vec![monster_class()],
        monstats2: vec![blank::<Monstats2>()],
        hitclass: vec![*b"none", *b"hth "],
    }
}

/// experience.txt: max level 3, thresholds 0, 100, 1500 for every class
/// (the kill's 100 experience reaches level 2);
/// charstats: the sorceress (class 1) gets 5 stat points per level (the
/// other per-level columns 0).
pub fn vitals() -> VitalsTables {
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
    let mut charstats = vec![blank::<Charstats>(); 7];
    charstats[1].statperlevel = 5;
    VitalsTables {
        charstats,
        experience: vec![row(3), row(0), row(100), row(1500)],
    }
}

/// AnimData with the fixture's two names: the
/// sorceress' cast, 8 frames at speed 256 with a missile event (2) on
/// frame 4; the monster's death, 4 frames at speed 256, no events.
pub fn anim_data() -> AnimData {
    let mut a = AnimData {
        buckets: vec![Vec::new(); animdata::BUCKETS],
    };
    let mut put = |name: &[u8; 8], frames, event: Option<usize>| {
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
    };
    put(PLAYER_SC, 8, Some(4));
    put(MONSTER_DT, 4, None);
    a
}

// ---- the fight ------------------------------------------------------------------------

const TOHIT: u16 = 19;
const LEVEL_STAT: u16 = 12;
const EXPERIENCE: u16 = 13;
const MINDAMAGE: u16 = 21;
const MAXDAMAGE: u16 = 22;

/// Most rows [`Fight`] fits in room B (sub-tile rows 2..38).
pub const MAX_ROWS: usize = 36;
/// Sub-tile column of the players (room B starts at 40, clear of the
/// missile barrier of room A).
const PLAYER_X: i32 = 42;
/// Sub-tile column of the monsters: 18 frames of flight.
const MONSTER_X: i32 = 60;
/// Each row fires every `PERIOD` frames, the rows staggered.
const PERIOD: i32 = 5;
/// Hits a monster takes before it dies (10 points of life each).
pub const HITS_TO_KILL: i32 = 8;

/// Rows of one player (sorceress, level 1, AR 300, with a client) firing an arrow every
/// [`PERIOD`] frames at one monster (class 0 of [`combat_tables`],
/// [`HITS_TO_KILL`] hits of life) across room B of [`Fx`], with the
/// e2e's tables, AnimData (death animation) and experience table. The
/// missile damage setup is not part of this wiring: set on each missile,
/// as the e2e does.
pub struct Fight {
    pub fx: Fx,
    pub rows: Vec<(UnitId, UnitId)>,
    pub fired: u32,
}

impl Fight {
    pub fn new(rows: usize) -> Self {
        assert!(rows <= MAX_ROWS, "at most {MAX_ROWS} rows");
        let mut fx = Fx::new();
        let h = fx.sim.hooks();
        h.tables = Arc::new(ActionTables {
            missiles: vec![arrow()],
            skills: skills(),
            combat: combat_tables(),
            levels: vec![blank::<Levels>(); 150],
            skill_modes: vec![[0; 8]],
            overlay_count: 0,
            monequip: Vec::new(),
            arena: Vec::new(),
        });
        h.anim_data = Some(Arc::new(anim_data()));
        h.vitals = Some(Arc::new(vitals()));
        h.x.names.insert((UnitType::Player, 10), *PLAYER_SC);
        h.x.names.insert((UnitType::Monster, 0), *MONSTER_DT);
        let mut v = Vec::with_capacity(rows);
        for i in 0..rows {
            let y = 2 + i as i32;
            let p = fx.spawn(UnitType::Player, 1, fx.b, PLAYER_X, y);
            let m = fx.spawn(UnitType::Monster, 0, fx.b, MONSTER_X, y);
            fx.stats(p, &[(TOHIT, 300), (LEVEL_STAT, 1)]);
            let life = HITS_TO_KILL * 2560;
            fx.stats(
                m,
                &[
                    (LEVEL_STAT, 1),
                    (EXPERIENCE, 100),
                    (st::MAXHP, life),
                    (st::HITPOINTS, life),
                ],
            );
            fx.sim.sys.units.get_mut(m).unwrap().flags |=
                unit_flag::IS_VALID_TARGET | unit_flag::CAN_BE_ATTACKED;
            fx.mark(m, bits::MONSTER);
            // Each player's client keeps the rooms active (rooms without
            // clients are removed after 11 passes).
            fx.game
                .lists
                .add_client(Some(p), None, client_state::IN_GAME);
            v.push((p, m));
        }
        Self {
            fx,
            rows: v,
            fired: 0,
        }
    }

    /// One tick: the rows due this frame fire, then the combined tick.
    pub fn tick(&mut self) {
        let frame = self.fx.game.frame;
        for (i, &(p, _)) in self.rows.iter().enumerate() {
            if (frame + i as i32) % PERIOD != 0 {
                continue;
            }
            let (_, y) = self.fx.sim.hooks().x.position(p);
            let params = MissileParams {
                owner: Some(p),
                origin: Some(p),
                class: 0,
                flags: param_flags::TARGET_ABSOLUTE,
                target_x: MONSTER_X,
                target_y: y,
                ..MissileParams::default()
            };
            let fx = &mut self.fx;
            let m = fx
                .sim
                .missiles(&mut fx.game, |g, cx| create_missile(g, cx, &params))
                .expect("missile")
                .expect("created");
            fx.stats(m, &[(MINDAMAGE, 2560), (MAXDAMAGE, 2560)]);
            self.fired += 1;
        }
        self.fx.tick();
    }

    /// Monsters whose life is 0.
    pub fn dead(&mut self) -> usize {
        let ms: Vec<UnitId> = self.rows.iter().map(|r| r.1).collect();
        ms.into_iter()
            .filter(|&m| self.fx.stat(m, st::HITPOINTS) <= 0)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bench's load is what it claims: missiles fly, hit and kill,
    /// with no wiring error, every kill within 60 ticks, and the run is
    /// deterministic.
    #[test]
    fn fight_kills_every_monster_cleanly_and_repeats() {
        let run = || {
            let mut f = Fight::new(MAX_ROWS);
            let mut last_kill = 0;
            for _ in 0..200 {
                let before = f.dead();
                f.tick();
                if f.dead() > before {
                    last_kill = f.fx.game.frame;
                }
            }
            f.fx.assert_clean();
            let xp: Vec<i32> = f
                .rows
                .clone()
                .into_iter()
                .map(|(p, _)| f.fx.stat(p, EXPERIENCE))
                .collect();
            (f.fired, f.dead(), xp, f.fx.game.frame, last_kill)
        };
        let a = run();
        assert_eq!(a.0, 200 * MAX_ROWS as u32 / PERIOD as u32);
        assert_eq!(a.1, MAX_ROWS, "every monster dies");
        assert!(a.2.iter().all(|&x| x == 100), "{:?}", a.2);
        // The bench's 60-tick case covers the whole fight.
        assert!(a.4 <= 60, "last kill on frame {}", a.4);
        assert_eq!(a, run());
    }
}
