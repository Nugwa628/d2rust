//! The Act II quest hooks the play host feeds once per tick
//! (`wired/quest_events.rs`): Radament's and the Summoner's AI calls and
//! the cube's Horadric Staff hook, raised as the pending seams queue
//! them: the path of a tick with no quest control lent (the lent path
//! runs the AI calls inside the think).
//!
//! Part of the world handler's test suite; uses the `trade_quests`
//! fixture.

use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::QuestEvent;

use super::trade_quests::Fx;

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

// Radament entering the sewers raises the lair quest.
#[test]
fn radament_in_the_sewers_raises_the_lair_quest() {
    let mut f = Fx::new(|_| {});
    let rad = monster(&mut f, 229);
    f.world().quests.record_mut(8).unwrap().not_intro = true;
    f.world().quests.record_mut(8).unwrap().state = 2;
    // Outside Sewers Level 3 the hook does nothing.
    f.world().rest.levels.insert(rad, 48);
    queue(&mut f, QuestEvent::RadamentActivated { unit: rad });
    frame(&mut f);
    assert_eq!(f.world().quests.record(8).unwrap().state, 2);
    // In it: state := 3, status := 2.
    f.world().rest.levels.insert(rad, 49);
    queue(&mut f, QuestEvent::RadamentActivated { unit: rad });
    frame(&mut f);
    let r = f.world().quests.record(8).unwrap();
    assert_eq!((r.state, r.status), (3, 2));
}

// The Summoner being seen starts quest chain 12.
#[test]
fn the_summoner_seen_starts_chain_12() {
    let mut f = Fx::new(|_| {});
    f.world().quests.record_mut(12).unwrap().not_intro = true;
    queue(&mut f, QuestEvent::SummonerActivated);
    frame(&mut f);
    let r = f.world().quests.record(12).unwrap();
    assert!(r.extra.a2.q5.seen);
    assert_eq!((r.state, r.status), (1, 2));
}

// An assembled Horadric Staff reaches the cube quest.
#[test]
fn an_assembled_staff_reaches_the_cube_quest() {
    let mut f = Fx::new(|_| {});
    let p = f.player;
    f.world().quests.record_mut(9).unwrap().not_intro = true;
    queue(&mut f, QuestEvent::StaffAssembled { player: p });
    frame(&mut f);
    let x = &f.world().quests.record(9).unwrap().extra.a2.q2;
    assert!(x.assembled);
}

// The preview cube records the quest item hooks for the host.
#[test]
fn the_preview_cube_records_quest_item_hooks_for_the_host() {
    use crate::adapters::handlers::items::ItemPending;
    use crate::adapters::handlers::world::PreviewCubePending;
    let mut c = PreviewCubePending::default();
    c.quest_item_hook(UnitId(1), UnitId(1), *b"hst ");
    assert_eq!(c.take_quest_items(), vec![(UnitId(1), *b"hst ")]);
    assert!(c.take_quest_items().is_empty());
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

/// A real item of the two-row table (0 `hst `, 1 `msf `), in the cursor
/// mode.
fn staff(f: &mut Fx, index: i32) -> UnitId {
    use d2_sim::items::tables::ItemRec;
    use d2_sim::items::{q, ItemRequest, ItemTables};
    use d2_sim::wiring::economy::ItemSpawn;
    f.world().tables = ItemTables {
        items: [b"hst ", b"msf "]
            .map(|code| ItemRec {
                code: *code,
                level: 1,
                ..ItemRec::default()
            })
            .to_vec(),
        ..ItemTables::default()
    };
    let mut rq = ItemRequest {
        item: index,
        format: 101,
        quality: q::NORMAL,
        ..ItemRequest::default()
    };
    let spawn = ItemSpawn {
        room: None,
        mode: 4,
        init_flags: 1,
    };
    let s = &mut f.h.game;
    s.world
        .with_economy(&mut s.game, &mut s.events, |econ, _| {
            econ.create_item(&mut rq, false, spawn)
        })
        .unwrap()
}

// The staff put in the orifice is handed in to the quest.
#[test]
fn the_staff_put_in_the_orifice_is_handed_in() {
    let mut f = Fx::new(|_| {});
    let p = f.player;
    f.world().quests.record_mut(13).unwrap().not_intro = true;
    let orifice = object(&mut f, 152);
    let og = f.guid(orifice);
    let (hst, msf) = (staff(&mut f, 0), staff(&mut f, 1));
    let (hg, mg) = (f.guid(hst), f.guid(msf));
    // Another item is refused (0x58 result 4); the staff is not handed in.
    queue(
        &mut f,
        QuestEvent::InsertItem {
            player: p,
            object: og,
            item: mg,
            action: 3,
        },
    );
    frame(&mut f);
    assert!(
        !f.world()
            .quests
            .record(13)
            .unwrap()
            .extra
            .a2
            .q6
            .staff_removed
    );
    // The staff: accepted, handed in, the lair objects are due.
    queue(
        &mut f,
        QuestEvent::InsertItem {
            player: p,
            object: og,
            item: hg,
            action: 3,
        },
    );
    frame(&mut f);
    let x = &f.world().quests.record(13).unwrap().extra.a2.q6;
    let got = (x.staff_removed, x.objects_update, x.timer_active);
    assert_eq!(got, (true, true, true), "{:?}", f.world().quests.faults);
    assert!(f.world().quests.faults.is_empty());
    assert_eq!(f.world().rest.object_modes[&orifice], 1);
}
