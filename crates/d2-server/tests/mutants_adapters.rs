//! Integration tests that close mutation-testing gaps (mutants of
//! `cargo mutants -p d2-server` that survived) in the adapters' public
//! surface of `d2-server`: the unit-target lookup (which units skip the act
//! test) and the skill handlers' routing and result codes.

use std::sync::{Arc, Mutex};

use d2_server::adapters::handlers::skills::{self, Call, Handled, SkillHost};
use d2_server::adapters::handlers::world::{WorldFault, WorldHost};
use d2_server::adapters::{PlayerData, PlayerFields, SimGame, UnitFacts, Unspecified};
use d2_server::buffers::ClientBuffers;
use d2_server::seams::*;
use d2_sim::game::Game;
use d2_sim::units::lists::client_state;
use d2_sim::units::{UnitId, UnitType};

/// A world host whose only system is the skill slot (`WorldHost::skill`);
/// `None`: no skill handlers.
#[derive(Default)]
struct SkillSlot(Option<Recorder>);

impl WorldHost<Unspecified> for SkillSlot {
    fn skill(&mut self, call: Call<'_, Unspecified>) -> Option<Handled> {
        self.0.as_mut()?.handle(call)
    }
    fn fault(&mut self, _: WorldFault) {}
}

type Sim = SimGame<Unspecified, SkillSlot>;

/// Act 0 with one active room: a player at (100, 100) for client 0, a
/// monster and an item, both in act 1 unless staged otherwise.
struct World {
    sim: Sim,
    player: UnitId,
    monster: UnitId,
    item: UnitId,
}

fn world() -> World {
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let room = game.lists.create_room(0).unwrap();
    game.lists.activate_room(room).unwrap();
    let player = game.spawn_unit(UnitType::Player, Some(room), true).unwrap();
    let monster = game
        .spawn_unit(UnitType::Monster, Some(room), false)
        .unwrap();
    let item = game.spawn_unit(UnitType::Item, None, false).unwrap();
    let mut sim: Sim = SimGame::with_events(game, Unspecified);
    sim.join(0, Some(player), Some(room), client_state::IN_GAME)
        .unwrap();
    sim.set_player(
        player,
        PlayerFields {
            gate: PlayerGate {
                mode: 1,
                uninterruptable: false,
            },
            data: Some(PlayerData { last_accept: 0 }),
        },
    );
    sim.set_unit(player, facts(0, None));
    World {
        sim,
        player,
        monster,
        item,
    }
}

fn facts(act: u8, owner: Option<UnitId>) -> UnitFacts {
    UnitFacts {
        act,
        pos: Pos { x: 100, y: 100 },
        owner,
    }
}

fn guid(sim: &Sim, u: UnitId) -> u32 {
    sim.game.lists.unit(u).unwrap().guid
}

/// Only an item the player owns skips the act test; an
/// item nobody owns and a monster the player owns do not.
#[test]
fn owned_item_skip_needs_an_owned_item() {
    let mut w = world();
    w.sim.set_unit(w.item, facts(1, None));
    w.sim.set_unit(w.monster, facts(1, Some(w.player)));
    let item = guid(&w.sim, w.item);
    let monster = guid(&w.sim, w.monster);
    assert_eq!(w.sim.unit_target(0, 4, item), UnitTarget::OtherAct);
    assert_eq!(w.sim.unit_target(0, 1, monster), UnitTarget::OtherAct);
    w.sim.set_unit(w.item, facts(1, Some(w.player)));
    assert_eq!(w.sim.unit_target(0, 4, item), UnitTarget::OwnedItem);
}

/// Result codes: the `d2-sim` handlers' 0–3 map one to one.
#[test]
fn handler_codes() {
    assert_eq!(skills::code(0), ResultCode::Done);
    assert_eq!(skills::code(1), ResultCode::Refused);
    assert_eq!(skills::code(2), ResultCode::Invalid);
    assert_eq!(skills::code(3), ResultCode::Malformed);
}

/// A skill host that records the ids it is called with.
struct Recorder(Arc<Mutex<Vec<u8>>>);

impl SkillHost<Unspecified> for Recorder {
    fn handle(&mut self, call: Call<'_, Unspecified>) -> Option<Handled> {
        self.0.lock().unwrap().push(call.msg[0]);
        Some(Handled {
            code: ResultCode::Refused,
            point_accept: None,
            resync: false,
            cant_do: false,
        })
    }
}

/// Only the ids the skill table marks handled reach the skill host:
/// 0x51 and 0x41 are `handlers::player`'s, and
/// a non-skill id (0x01) is not routed here.
#[test]
fn skill_routing() {
    let mut w = world();
    let calls = Arc::new(Mutex::new(Vec::new()));
    w.sim.world.0 = Some(Recorder(calls.clone()));
    let mut out = ClientBuffers::new();
    for id in [0x01, 0x41, 0x51] {
        assert!(!skills::handled(id));
        let msg = [id, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(skills::handle(&mut w.sim, 0, &msg, &mut out), None);
    }
    assert!(calls.lock().unwrap().is_empty());
    assert!(skills::handled(0x3C));
    let select = [0x3C, 5, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
    assert_eq!(
        skills::handle(&mut w.sim, 0, &select, &mut out),
        Some(ResultCode::Refused)
    );
    assert_eq!(*calls.lock().unwrap(), vec![0x3C]);
    // Without a host the handled id stays a stub too.
    w.sim.world.0 = None;
    assert_eq!(skills::handle(&mut w.sim, 0, &select, &mut out), None);
}
