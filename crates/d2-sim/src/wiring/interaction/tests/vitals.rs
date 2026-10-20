//! Tests for vitals ↔ stats: a player created, a kill granting experience and a
//! level-up, stat points spent and the regeneration that follows, all on
//! the real unit records and stat lists.

use super::*;
use crate::combat::vitals::{
    handle_add_stat_point, init_player_stats, stat as vs, VitalsTables, VitalsUnits,
};
use crate::units::dispatch::player_regen;
use crate::wiring::interaction::vitals::kill_experience;
use crate::wiring::interaction::{VitalsRest, VitalsView};
use d2_data::tables::{Charstats, Experience};

const SORCERESS: u32 = 1;

#[derive(Default)]
struct Notes(Vec<String>);

impl VitalsRest for Notes {
    fn refresh(&mut self, u: UnitId) {
        self.0.push(format!("refresh {}", u.0));
    }
    fn level_up_notify(&mut self, u: UnitId) {
        self.0.push(format!("notify {}", u.0));
    }
    fn level_up_event(&mut self, u: UnitId) {
        self.0.push(format!("event12 {}", u.0));
    }
}

/// The Sorceress row of the character stats (other classes the same)
/// and a synthetic experience table: level `L` → 500·L² (level 1 →
/// 500), `MaxLvl` 99.
fn tables() -> VitalsTables {
    let mut c = Charstats::decode(&[0u8; Charstats::SIZE]);
    (c.str, c.dex, c.int, c.vit, c.stamina, c.hpadd) = (10, 25, 35, 10, 74, 30);
    (c.lifeperlevel, c.staminaperlevel, c.manaperlevel) = (4, 4, 8);
    (c.lifepervitality, c.staminapervitality, c.manapermagic) = (8, 4, 8);
    c.statperlevel = 5;
    let row = |v: u32| {
        let mut e = Experience::decode(&[0u8; Experience::SIZE]);
        (e.amazon, e.sorceress, e.necromancer, e.paladin) = (v, v, v, v);
        (e.barbarian, e.druid, e.assassin) = (v, v, v);
        e
    };
    let mut experience = vec![row(99)];
    experience.extend((0..=99u32).map(|l| row(500 * l * l)));
    VitalsTables {
        charstats: vec![c; 7],
        experience,
    }
}

fn view<'a>(w: &'a mut World, n: &'a mut Notes) -> VitalsView<'a, Hooks, Notes> {
    VitalsView {
        units: &w.units,
        stats: &mut w.stats,
        hooks: &mut w.hooks,
        rest: n,
    }
}

#[test]
fn kill_grants_experience_and_a_level_up_on_real_stats() {
    let mut w = World::new(false);
    let t = tables();
    let mut n = Notes::default();
    let player = w.spawn(UnitType::Player, SORCERESS);
    let monster = w.spawn(UnitType::Monster, 0);
    init_player_stats(&mut view(&mut w, &mut n), &t, player, 0);
    // Sorceress created.
    assert_eq!(w.stat(player, st::MAXHP), 10240);
    assert_eq!(w.stat(player, st::MAXMANA), 8960);
    assert_eq!(w.stat(player, st::MAXSTAMINA), 18944);
    assert_eq!(w.stat(player, st::LEVEL), 1);
    assert_eq!(w.stat(player, vs::NEXTEXP), 500);
    // A level-1 monster worth 600 experience (stat 13, monster init).
    w.set(monster, &[(st::EXPERIENCE, 600), (st::LEVEL, 1)]);
    w.set(player, &[(st::LIFE, 5000)]);
    let gain = kill_experience(&mut view(&mut w, &mut n), &t, player, monster);
    // Same level → factor 256 → the whole 600.
    assert_eq!(gain, 600);
    assert_eq!(w.stat(player, st::EXPERIENCE), 600);
    // Level 2 (600 ≥ 500), one level's worth of per-level gains.
    assert_eq!(w.stat(player, st::LEVEL), 2);
    assert_eq!(w.stat(player, vs::NEXTEXP), 2000);
    assert_eq!(w.stat(player, st::MAXHP), 10240 + (4 << 6));
    assert_eq!(w.stat(player, st::LIFE), 10240 + (4 << 6));
    assert_eq!(w.stat(player, st::MAXMANA), 8960 + (8 << 6));
    assert_eq!(w.stat(player, st::MANA), 8960 + (8 << 6));
    assert_eq!(w.stat(player, st::MAXSTAMINA), 18944 + (4 << 6));
    assert_eq!(w.stat(player, vs::STATPTS), 5);
    assert_eq!(w.stat(player, vs::NEWSKILLS), 1);
    assert_eq!(
        n.0,
        [
            format!("notify {}", player.0),
            format!("event12 {}", player.0)
        ]
    );
    // A monster killing gets nothing (players only).
    assert_eq!(
        kill_experience(&mut view(&mut w, &mut n), &t, monster, player),
        0
    );
    w.assert_clean();
}

#[test]
fn stat_points_and_regeneration_share_the_real_lists() {
    let mut w = World::new(false);
    let t = tables();
    let mut n = Notes::default();
    let player = w.spawn(UnitType::Player, SORCERESS);
    init_player_stats(&mut view(&mut w, &mut n), &t, player, 0);
    w.set(player, &[(vs::STATPTS, 5)]);
    // C→S 0x3A: two points (count − 1 = 1) into vitality (3).
    let r = handle_add_stat_point(&mut view(&mut w, &mut n), &t, player, &[0x3A, 3, 1]);
    assert_eq!(r, 0);
    assert_eq!(w.stat(player, vs::STATPTS), 3);
    assert_eq!(w.stat(player, vs::VITALITY), 12);
    assert_eq!(w.stat(player, st::MAXHP), 10240 + 2 * (8 << 6));
    assert_eq!(w.stat(player, st::LIFE), 10240 + 2 * (8 << 6));
    assert_eq!(w.stat(player, st::MAXSTAMINA), 18944 + 2 * (4 << 6));
    // Energy through the same handler: mana and its maximum.
    let r = handle_add_stat_point(&mut view(&mut w, &mut n), &t, player, &[0x3A, 1, 0]);
    assert_eq!(r, 0);
    let max_mana = 8960 + (8 << 6);
    assert_eq!(w.stat(player, st::MAXMANA), max_mana);
    assert_eq!(
        view(&mut w, &mut n).max_mana(player),
        w.stats.max_mana(player)
    );
    // Regeneration (timer event 3) on the same lists: mana from 0 by
    // max(max mana / (ManaRegen · 25), 1) = 9472 / 100 = 94.
    w.set(player, &[(st::MANA, 0)]);
    let mut sim = Sim {
        game: &mut w.game,
        units: &mut w.units,
        stats: &mut w.stats,
        data: &w.data,
    };
    player_regen(&mut sim, &mut w.hooks, player, 0, 0).unwrap();
    assert_eq!(w.stat(player, st::MANA), max_mana / 100);
    // Spending without points fails with 2 and changes nothing.
    w.set(player, &[(vs::STATPTS, 0)]);
    let r = handle_add_stat_point(&mut view(&mut w, &mut n), &t, player, &[0x3A, 0, 0]);
    assert_eq!(r, 2);
    w.assert_clean();
}
