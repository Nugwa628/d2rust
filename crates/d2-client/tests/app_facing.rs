//! B025: a monster that attacks the player turns toward it (the unit group
//! of the monster mode machine), toward where the player
//! stands: the walk prediction's position, not the last point the server
//! gave the local player (which the walk leaves behind).
use d2_client::bridge::critter_path::{direction, Who};
use d2_client::bridge::BridgeResource;
use d2_sim::monsters::ai::{install, AiControl};
use d2_sim::tick::events::event;
use d2_sim::units::UnitId;

use crate::real_rig;

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn an_attacking_monster_turns_toward_the_player() {
    let mut r = real_rig::Rig::new("sorceress", &[]);
    r.step(2);
    // Out of town and some way from the join point, so the model's point
    // of the local player is far from where it stands.
    r.leave_town();
    let m: UnitId = r.spawn_monster_at(1, 1);
    r.with(move |sim, _| {
        let a = &mut sim.events.action;
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
    let tables = d2_sim::path::tables::PathTables::spec().unwrap();
    let mut checked = 0;
    for _ in 0..40 {
        r.step(1);
        let (px, py) = r.pos();
        let b = &r.app.world().resource::<BridgeResource>().0;
        let w = b.world();
        // The monster nearest the player: the one just spawned.
        let Some(mon) = w
            .units
            .values()
            .filter(|u| u.key.unit_type == 1)
            .min_by_key(|u| {
                let c = u.cell();
                (i32::from(c.0) - px).abs() + (i32::from(c.1) - py).abs()
            })
        else {
            continue;
        };
        if !matches!(mon.mode, 4 | 5) {
            continue;
        }
        let (mx, my) = mon.cell();
        let (_, want) = d2_sim::path::walk::geom::direction_vector(
            &tables,
            (
                (u32::from(mx) << 16) | 0x8000,
                (u32::from(my) << 16) | 0x8000,
            ),
            ((px as u32) << 16 | 0x8000, (py as u32) << 16 | 0x8000),
        );
        let dir = direction(w, Who::server(mon.key)).expect("the monster's path");
        // The turn takes a few frames (one step per update);
        // once turned it stays on the player for the rest of the attack.
        if checked >= 8 {
            assert_eq!(dir, want & 63, "faces the player");
        }
        checked += 1;
    }
    assert!(checked >= 9, "the monster attacked ({checked} frames)");
}
