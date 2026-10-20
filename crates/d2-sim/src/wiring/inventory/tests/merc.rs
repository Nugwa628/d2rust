//! Tests for the 0x61 give on the lent hireling lists: the room test's
//! argument order (which unit's room list is scanned for the other's
//! room) and the swap's update-list and refresh steps.

use super::*;
use crate::world::hirelings::{HirelingState, PetNode};

/// A merc monster in `room` with a living node in the player's list.
/// The player alive (mode 1) and free to give.
fn ready(w: &mut World) {
    w.units.get_mut(w.player).unwrap().mode = 1;
    w.rest.merc_ready = true;
}

fn add_merc(w: &mut World, room: RoomId) -> Owner {
    if w.data.monsters.is_empty() {
        w.data.monsters.push(crate::units::hooks::MonsterInfo {
            enabled: true,
            aidel: [15; 3],
            moves: 0,
            mode_chart: false,
        });
    }
    let m = w.alloc(UnitType::Monster, 0);
    if room != w.room {
        w.game.lists.change_room(m, room).unwrap();
    }
    w.set_stat(m, 0, 10);
    w.set_stat(m, 2, 10);
    let guid = w.units.get(m).unwrap().guid;
    let p = w.player;
    let st = w.state.hirelings.get_or_insert_with(HirelingState::default);
    st.list_mut(p).nodes.push(PetNode {
        dead: false,
        guid,
        seed: 0,
        name: 0,
        id: 0,
    });
    Owner::monster(guid)
}

/// The player's room list is scanned for the hireling's
/// room. Asymmetric adjacency: the player's room lists the merc's room,
/// not the reverse -> the give proceeds; the reverse -> it does not.
#[test]
fn room_test_scans_the_players_room_list() {
    let mut w = World::new();
    let r2 = w.game.lists.create_room(0).unwrap();
    let merc = add_merc(&mut w, r2);
    ready(&mut w);
    let me = w.me();
    let r1 = w.room;
    // Neither lists the other (the room itself is not implied here).
    assert_eq!(w.desk(|d| d.lent_owns_hireling(me, merc)), Some(false));
    w.game.lists.room_mut(r1).unwrap().adjacent.push(r2);
    assert_eq!(w.desk(|d| d.lent_owns_hireling(me, merc)), Some(true));
    w.game.lists.room_mut(r1).unwrap().adjacent.clear();
    w.game.lists.room_mut(r2).unwrap().adjacent.push(r1);
    assert_eq!(w.desk(|d| d.lent_owns_hireling(me, merc)), Some(false));

    // Through the 0x61: the helm is equipped on the merc only when the
    // player's list holds the merc's room.
    let c = w.cursor_item(CAP);
    w.game.lists.room_mut(r2).unwrap().adjacent.clear();
    let p = w.pguid();
    let _ = w.desk(|d| crate::items::moves::handle(d, p, &msg(0x61, &[0])[..3]).unwrap());
    assert!(w.state.cursor_of(w.player).is_some());
    w.game.lists.room_mut(r1).unwrap().adjacent.push(r2);
    let _ = w.desk(|d| crate::items::moves::handle(d, p, &msg(0x61, &[0])[..3]).unwrap());
    assert_eq!(w.state.cursor_of(w.player), None);
    let _ = c;
}

/// A give onto an occupied merc slot leaves old's GUID in the
/// merc's update list and queues the merc (flags 2 |= 0x1).
#[test]
fn swap_queues_old_item_and_merc() {
    let mut w = World::new();
    let room = w.room;
    let merc = add_merc(&mut w, room);
    ready(&mut w);
    w.game.lists.room_mut(room).unwrap().adjacent.push(room);
    let mu = w
        .game
        .lists
        .find_unit(UnitType::Monster, merc.guid)
        .unwrap();
    let p = w.pguid();
    let first = w.cursor_item(CAP);
    let _ = w.desk(|d| crate::items::moves::handle(d, p, &msg(0x61, &[0])[..3]).unwrap());
    assert_eq!(w.state.cursor_of(w.player), None);
    let old = w.state.body_items(mu);
    assert_eq!(old.len(), 1);
    let _ = first;
    let old_guid = w.units.get(old[0]).unwrap().guid;
    // Reset what the first give left behind.
    w.state.inventories.get_mut(&mu).unwrap().take_updates();
    w.units.get_mut(mu).unwrap().flags2 = 0;
    w.game.lists.clear_update_queue(room).unwrap();
    assert!(!w.game.lists.update_queue(room).contains(&mu));

    let _second = w.cursor_item(CAP);
    let _ = w.desk(|d| crate::items::moves::handle(d, p, &msg(0x61, &[0])[..3]).unwrap());
    assert!(w.state.inventories[&mu].update_list().contains(&old_guid));
    assert_eq!(w.units.get(mu).unwrap().flags2 & 1, 1);
    assert!(w.game.lists.update_queue(room).contains(&mu));
}
