//! A monster attacks the player in the play preview, headless on the
//! user's install (`real_rig`): a zombie (`monstats` row 5, its own AI,
//! skills and AnimData) next to the player in the Blood Moor notices it,
//! attacks, the hit runs the server's damage path and the player's life
//! reaches 0 (then q-death: DT, the screen).
//!
//! Staging (test inputs): the player has 10 life, the zombie a to-hit of
//! 1000 and 20 damage, so one hit kills. Some parts are provisional.

use d2_client::app::death::{add_death, DeathScreen};
use d2_client::app::single_player;
use d2_client::bridge::modes::player_mode;
use d2_client::bridge::BridgeResource;
use d2_sim::missiles::unit_flag as flags;
use d2_sim::monsters::ai::{install, AiControl};
use d2_sim::stats::stat;
use d2_sim::tick::events::event;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::UnitType;

use crate::real_rig;

const MINDAMAGE: u16 = 21;
const MAXDAMAGE: u16 = 22;
const TOHIT: u16 = 19;

fn local_mode(r: &real_rig::Rig) -> u32 {
    r.app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .local()
        .expect("local")
        .mode
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_monster_next_to_the_player_attacks_until_the_player_dies() {
    attacks_until_the_player_dies(single_player::DEFAULT_SEED);
}

/// Another game seed: the attacks go on until the player dies whatever
/// the first one rolls (unit flag 0x40, set by a do, is cleared by the
/// next Attack start).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_missed_first_attack_is_followed_by_more_attacks() {
    attacks_until_the_player_dies(9);
}

fn attacks_until_the_player_dies(seed: u32) {
    let mut r = real_rig::Rig::with_seed("sorceress", &[], seed);
    add_death(&mut r.app);
    r.step(2);
    assert_eq!(local_mode(&r), player_mode::TOWN_NEUTRAL, "joined");
    // Out of town (the AI skips players in a town room).
    r.leave_town();
    // A zombie one sub-tile from the player, its AI installed as monster
    // creation does, and a player one hit kills.
    r.with(move |sim, p| {
        let a = &mut sim.events.action;
        let (px, py) = a.sys.hooks.path_position(p);
        let room = sim.game.lists.unit(p).and_then(|e| e.room()).expect("room");
        let req = AllocRequest {
            ty: UnitType::Monster,
            class: real_rig::ZOMBIE,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: false,
        };
        let m = a
            .with(&mut sim.game, |g, v| v.allocate(g, &req, px + 1, py))
            .expect("monster");
        a.with(&mut sim.game, |_, v| {
            v.set_base(p, stat::HITPOINTS, 2560);
            v.set_base(p, stat::MAXHP, 2560);
            v.set_base(m, stat::LEVEL, 1);
            v.set_base(m, stat::MAXHP, 25600);
            v.set_base(m, stat::HITPOINTS, 25600);
            v.set_base(m, TOHIT, 1000);
            v.set_base(m, MINDAMAGE, 5120);
            v.set_base(m, MAXDAMAGE, 5120);
        });
        a.sys.units.get_mut(m).unwrap().flags |= flags::IS_VALID_TARGET | flags::CAN_BE_ATTACKED;
        a.ai(&mut sim.game, |g, cx| {
            cx.store.entry(m).control = Some(AiControl::default());
            install(g, cx, m, 0);
        })
        .expect("ai");
        let at = sim.game.frame + 1;
        sim.game
            .schedule_event(m, u32::from(event::AI_THINK), at, None, 0, 0)
            .unwrap();
    });
    // The AI thinks, the zombie attacks, the hit goes through the
    // server's damage path, the player's life reaches 0.
    r.step(120);
    let errors = r.errors();
    let life = r.with(|sim, p| {
        sim.events
            .action
            .with(&mut sim.game, |_, v| v.stat(p, stat::HITPOINTS))
    });
    assert!(life <= 0, "the player's life is {life} ({errors})");
    // DT, or already DD when the install's death animation has ended.
    let mode = local_mode(&r);
    assert!(
        mode == player_mode::DEATH || mode == player_mode::DEAD,
        "the player died: mode {mode}"
    );
    assert!(r.app.world().resource::<DeathScreen>().active);
}
