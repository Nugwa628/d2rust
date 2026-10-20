use super::*;
use crate::units::UnitType;

fn game() -> (Game, UnitId, UnitId, crate::units::RoomId) {
    let mut g = Game::new();
    g.lists.ensure_act(0).unwrap();
    let room = g.lists.create_room(0).unwrap();
    g.lists.activate_room(room).unwrap();
    let p = g.spawn_unit(UnitType::Player, Some(room), true).unwrap();
    let q = g.spawn_unit(UnitType::Player, Some(room), true).unwrap();
    (g, p, q, room)
}

/// The 0x2C bytes (`server-messages.tsv` row 0x2C):
/// `2C`, type u8 @1, GUID u32 @2, event u16 @6, 8 bytes.
#[test]
fn message_layout() {
    assert_eq!(
        play_sound_message(0, 0x1234_5678, 0x0104),
        [0x2C, 0, 0x78, 0x56, 0x34, 0x12, 0x04, 0x01]
    );
}

/// The last event and target before the flush win; the unit
/// is queued for update. Step 2: target none → every client's player;
/// target P → P's client only. Removal clears the slot.
#[test]
fn queue_overwrite_and_target() {
    let (mut g, p, q, room) = game();
    let _ = g.lists.unqueue_update(p);
    queue_sound(&mut g, p, 4, None).unwrap();
    queue_sound(&mut g, p, 19, Some(p)).unwrap();
    assert!(g.lists.update_queue(room).contains(&p));
    assert_eq!(
        g.sounds.get(p),
        Some(SoundSlot {
            event: 19,
            target: Some(p)
        })
    );
    let guid = g.lists.unit(p).unwrap().guid;
    assert_eq!(
        sound_message(&g, p, p),
        Some(play_sound_message(0, guid, 19))
    );
    assert_eq!(sound_message(&g, p, q), None);
    queue_sound(&mut g, p, 2, None).unwrap();
    assert_eq!(
        sound_message(&g, p, q),
        Some(play_sound_message(0, guid, 2))
    );
    // No slot: nothing.
    assert_eq!(sound_message(&g, q, q), None);
    g.remove_unit(p).unwrap();
    assert!(g.sounds.is_empty());
}
