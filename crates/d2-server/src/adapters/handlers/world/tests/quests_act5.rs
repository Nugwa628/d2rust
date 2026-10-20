//! The Act V quest hooks the play host feeds once per tick
//! (`wired/quest_events.rs`): Shenk's, Nihlathak's,
//! the Ancients' and the Baal crab's AI calls and Anya's portal request,
//! raised as the pending seams queue them: the path of a tick with no
//! quest control lent (the lent path runs them inside the think,
//! `QuestObjectHost::ai_quest_call` / `anya_open_portal`).
//!
//! Part of the world handler's test suite; uses the `trade_quests`
//! fixture.

use std::sync::Arc;

use d2_data::tables::Objects;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::QuestEvent;
use d2_sim::world::objects::ObjectTables;

use super::trade_quests::Fx;
use super::waypoints::blank;

fn monster(f: &mut Fx, class: u32) -> UnitId {
    let req = AllocRequest {
        ty: UnitType::Monster,
        class,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    let s = &mut f.h.game;
    s.events
        .with(&mut s.game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap()
}

fn queue(f: &mut Fx, e: QuestEvent) {
    f.h.game.events.hooks().x.quest_events.push(e);
}

fn frame(f: &mut Fx) {
    f.h.clock.0 += 40;
    assert!(f.h.frame().unwrap().ticked);
}

// Shenk activating raises the siege status.
#[test]
fn shenk_activated_raises_the_siege_status() {
    let mut f = Fx::new(|_| {});
    let shenk = monster(&mut f, 120);
    f.world().quests.record_mut(31).unwrap().not_intro = true;
    // Not linked to chain 31: not the siege boss, nothing happens.
    queue(&mut f, QuestEvent::ShenkActivated { unit: shenk });
    frame(&mut f);
    assert_eq!(f.world().quests.record(31).unwrap().status, 0);
    // Linked by monster init (superunique 42 → chain 31): status 2.
    f.world().rest.chains.insert(shenk, Default::default());
    queue(
        &mut f,
        QuestEvent::Link {
            unit: shenk,
            chain: 31,
        },
    );
    queue(&mut f, QuestEvent::ShenkActivated { unit: shenk });
    frame(&mut f);
    assert_eq!(f.world().quests.record(31).unwrap().status, 2);
}

// Nihlathak activating raises status three.
#[test]
fn nihlathak_activated_raises_status_three() {
    let mut f = Fx::new(|_| {});
    f.world().quests.record_mut(34).unwrap().not_intro = true;
    queue(&mut f, QuestEvent::NihlathakActivated);
    frame(&mut f);
    assert_eq!(f.world().quests.record(34).unwrap().status, 3);
}

// The Ancients' AI call disarms the fight.
#[test]
fn the_ancients_ai_disarms_the_fight() {
    let mut f = Fx::new(|_| {});
    f.world().quests.record_mut(35).unwrap().extra.a5.q5.armed = true;
    queue(&mut f, QuestEvent::AncientsDisarm);
    frame(&mut f);
    assert!(!f.world().quests.record(35).unwrap().extra.a5.q5.armed);
}

// The Baal crab call opens the Worldstone chamber warp.
#[test]
fn the_baal_crab_opens_the_worldstone_chamber() {
    let mut f = Fx::new(|_| {});
    f.world().quests.record_mut(36).unwrap().not_intro = true;
    assert!(!d2_sim::world::quests::act5::q6::chamber_warp_open(
        &f.world().quests
    ));
    queue(&mut f, QuestEvent::BaalToStairs);
    frame(&mut f);
    let q = &f.world().quests;
    assert!(d2_sim::world::quests::act5::q6::chamber_warp_open(q));
    assert_eq!(q.record(36).unwrap().status, 3);
}

// Anya's portal request is honoured only when the quest wants it.
#[test]
fn anya_asks_for_the_temple_portal_only_when_wanted() {
    let mut f = Fx::new(|_| {});
    let anya = monster(&mut f, 229);
    queue(&mut f, QuestEvent::AnyaOpenPortal { unit: anya });
    frame(&mut f);
    // Anya's portal flag unset: nothing is made and the flag stays clear.
    assert!(!f.world().quests.record(34).unwrap().extra.a5.q4.anya_portal);
}

const ALTAR: u32 = 546;
const KEEP_DOOR: u32 = 547;
const LAST_PORTAL: u32 = 565;

/// objects.txt rows 0–565 with the Act V quest init functions:
/// altar 72, keep door 73, last portal 77.
fn object_fixture() -> Fx {
    let mut rows = vec![blank::<Objects>(); 566];
    rows[ALTAR as usize].initfn = 72;
    rows[KEEP_DOOR as usize].initfn = 73;
    rows[LAST_PORTAL as usize].initfn = 77;
    let tables = ObjectTables {
        objects: rows,
        shrines: Vec::new(),
        levels: Vec::new(),
        objgroup: Vec::new(),
        leveldefs: Vec::new(),
    };
    let mut fx = Fx::new(|_| {});
    fx.h.game.events.create_objects(Arc::new(tables));
    fx.h.game.events.route_quest_objects();
    fx
}

fn object(f: &mut Fx, class: u32) -> UnitId {
    let req = AllocRequest {
        ty: UnitType::Object,
        class,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 0,
        allied: false,
    };
    let s = &mut f.h.game;
    s.events
        .with(&mut s.game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap()
}

fn mode(f: &mut Fx, u: UnitId) -> u32 {
    f.h.game.events.sys.units.get(u).unwrap().mode
}

// The Act V object init functions run on the quest control.
#[test]
fn act_v_object_inits_run_on_the_quest_control() {
    let mut f = object_fixture();
    {
        let q = &mut f.world().quests;
        q.record_mut(35).unwrap().extra.a5.q5.done_before = true;
        q.record_mut(35).unwrap().extra.a5.q5.altar_mode = 1;
    }
    let door = object(&mut f, KEEP_DOOR);
    let altar = object(&mut f, ALTAR);
    let portal = object(&mut f, LAST_PORTAL);
    // Queued, not run, by the allocation.
    assert_eq!(mode(&mut f, door), 0);
    frame(&mut f);
    // Door 547: the Ancients done before → mode 2.
    assert_eq!(mode(&mut f, door), 2);
    // Altar 546: mode := the record's altar mode.
    assert_eq!(mode(&mut f, altar), 1);
    // Last portal 565: mode := the record's last-portal mode (1), which then := 2.
    assert_eq!(mode(&mut f, portal), 1);
    assert_eq!(
        f.world()
            .quests
            .record(36)
            .unwrap()
            .extra
            .a5
            .q6
            .last_portal_mode,
        2
    );
}
