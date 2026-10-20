// D2RUST's own: the local player on its first move.
//! A freshly loaded character's first click on the floor: the click puts
//! the local player in the walk (or run) mode inside the frame pass, after
//! that pass loaded the unit art. The walk's COF and component files must
//! be resident for that tick's draw, or the player is skipped for a tick
//! (owner report: the character flickers right after it first moves).

use d2_client::app::autoplay_host::{Host, HostArgs};
use d2_client::app::control::protocol::parse_command;
use d2_client::bridge::BridgeResource;
use d2_client::scene::ItemTag;
use d2_client::world_view::WorldViewState;

/// The draw items of the local player in the last built frame (its body's
/// components and shadows), and its model position.
fn player_drawn(host: &mut Host) -> (usize, Option<(u16, u16)>) {
    let w = host.app_mut().world();
    let world = w.resource::<BridgeResource>().0.world();
    let local = world.local().expect("local player");
    let (guid, at) = (local.key.guid, local.position);
    let drawn = w
        .resource::<WorldViewState>()
        .last_tags
        .iter()
        .filter(|t| **t == ItemTag::Unit(guid))
        .count();
    (drawn, at)
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_first_click_to_walk_draws_the_player_on_every_tick() {
    let mut host = Host::start(&HostArgs {
        new: Some(("sorceress".into(), "First".into())),
        seed: Some(7),
        ..Default::default()
    })
    .unwrap();
    host.step(60);
    let (standing, start) = player_drawn(&mut host);
    assert!(
        standing > 4,
        "standing: the player is drawn ({standing} items)"
    );
    // A drawn body has most of its standing items; a skipped one only
    // keeps the few items drawn outside the body (2).
    let body = |n: usize| 2 * n >= standing;
    // One pass: the pointer moves to the floor, presses and releases (a
    // scripted click).
    for line in ["move 520 250", "press l 520 250", "release l 520 250"] {
        host.run(parse_command(line).unwrap()).unwrap();
    }
    host.step(1);
    let (n, _) = player_drawn(&mut host);
    assert!(body(n), "the click's pass: the player is drawn ({n} items)");
    for n in 0..60 {
        host.step(1);
        let (items, _) = player_drawn(&mut host);
        assert!(
            body(items),
            "tick {n} after the click: the player is drawn ({items} items)"
        );
    }
    assert_ne!(
        player_drawn(&mut host).1,
        start,
        "the click moved the player"
    );
}
