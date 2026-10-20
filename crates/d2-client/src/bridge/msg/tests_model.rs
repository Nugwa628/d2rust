//! Test vectors of the client model (bytes from a recorded packet trace
//! unless marked synthetic).

use super::super::check::{check, Checked};
use super::super::world::{
    room_of_point, update_order, ActLoad, ActiveRoom, ClientUnit, ClientWorld, LevelRow,
    ModelInputs, MonsterClass, PetRecord, RoomSight, UnitKey, VisibleFn, MISSILE, MONSTER, OBJECT,
    PLAYER,
};
use super::support::{hex, Model};

const AC_1_6: &str = "ac 06 00 00 00 9a 00 1a 12 b9 11 80 0e 01";
const STOP_1_6: &str = "6d 06 00 00 00 1a 12 b9 11 80";
const M6: UnitKey = UnitKey::new(MONSTER, 6);
const P1: UnitKey = UnitKey::new(PLAYER, 1);

/// A model whose tables have `monstats` rows 0..=154, each with a
/// `monstats2` row and no components.
fn with_monsters() -> Model {
    let mut m = Model::default();
    m.inputs.tables.monsters = vec![Some(MonsterClass::default()); 155];
    m
}

/// 0x59 for player 1 ("werwer", class 1) at (x, y).
fn assign_player(x: u16, y: u16) -> Vec<u8> {
    let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72");
    b.resize(0x16, 0);
    b.extend_from_slice(&x.to_le_bytes());
    b.extend_from_slice(&y.to_le_bytes());
    b
}

#[test]
fn session_flags_and_load_successful() {
    let mut m = Model::default();
    m.hex("01 00 04 00 10 00 01 00");
    assert_eq!(
        (m.w.difficulty, m.w.game_flags, m.w.expansion, m.w.ladder),
        (0, 0x0010_0004, 1, 0)
    );
    let before = m.w.clone();
    m.hex("00");
    assert_eq!(m.w, before);
    m.hex("02");
    assert_eq!(m.w.outgoing, vec![vec![0x6B]]);
    assert!(m.log.rejected.is_empty());
    assert_eq!(m.log.handled, 3);
}

#[test]
fn local_player_from_0x0b() {
    let mut m = Model::default();
    m.recv(&assign_player(0, 0)).hex("0b 00 01 00 00 00");
    assert_eq!(m.w.local_player, Some(P1));
    // Randomness step 2: one step on {1, 666}.
    assert_eq!(m.unit(P1).seed, Some((0x6AC6_935F, 0)));
    // An unknown key: unchanged.
    m.hex("0b 00 09 00 00 00");
    assert_eq!(m.w.local_player, Some(P1));
    // Cleared only by freeing that unit; a replacing add frees it.
    m.recv(&assign_player(0, 0));
    assert_eq!(m.w.local_player, None);
    m.hex("0b 00 01 00 00 00").hex("0a 00 01 00 00 00");
    assert_eq!(m.w.local_player, None);
    assert!(m.w.units.is_empty());
}

#[test]
fn load_act_and_rooms_in_sight() {
    let mut m = Model::default();
    // Without an act: fatal 0x58A / 0x59E (handler errors), no change.
    m.hex("07 a0 03 88 03 01").hex("08 d0 03 60 04 01");
    assert_eq!(
        m.rejected(),
        [
            (0x07, "fatal assert 0x58A".to_owned()),
            (0x08, "fatal assert 0x59E".to_owned())
        ]
    );
    assert!(m.w.rooms_in_sight.is_empty());
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    assert_eq!(
        m.w.act,
        Some(ActLoad {
            act: 0,
            init_seed: 0x1038_88C4,
            town_level: 1,
            f8: 0x9FE0_D161
        })
    );
    m.hex("07 a0 03 88 03 01").hex("08 d0 03 60 04 01");
    assert_eq!(
        m.w.rooms_in_sight,
        [
            RoomSight {
                show: true,
                level: 1,
                x: 0x3A0,
                y: 0x388
            },
            RoomSight {
                show: false,
                level: 1,
                x: 0x3D0,
                y: 0x460
            }
        ]
    );
}

#[test]
fn load_complete_unload_and_exit() {
    let mut m = Model::default();
    // No local player: fatal 0x527.
    m.hex("04");
    assert_eq!(m.rejected(), [(0x04, "fatal assert 0x527".to_owned())]);
    assert!(!m.w.in_game);
    // A local player created at (0, 0) has no room: still fatal.
    m.recv(&assign_player(0, 0))
        .hex("0b 00 01 00 00 00")
        .hex("04");
    assert_eq!(m.log.rejected.len(), 2);
    // Placed by 0x15: in game.
    m.hex("15 00 01 00 00 00 41 12 c4 11 01").hex("04");
    assert!(m.w.in_game && !m.w.unloaded);
    m.hex("05");
    assert!(!m.w.in_game && m.w.unloaded);
    m.hex("06");
    assert!(m.w.exit_requested);
    assert_eq!(m.log.rejected.len(), 2);
}

#[test]
fn unit_message_is_queued_then_applied_in_the_update_pass() {
    let mut m = with_monsters();
    m.recv(&[hex(AC_1_6), hex(STOP_1_6)].concat());
    assert_eq!(m.log.queued, 1);
    let u = m.unit(M6);
    assert_eq!(u.queue, vec![hex(STOP_1_6)]);
    // Not applied at receive.
    assert_eq!(u.last_mode_request, None);
    assert_eq!(u.stat(328), 0x23D3);
    assert_eq!(m.drain(), 1);
    let u = m.unit(M6);
    assert!(u.queue.is_empty());
    assert_eq!(u.stat(328), 0x23D4);
    assert_eq!(
        u.last_mode_request.map(|r| (r.code, r.record)),
        Some((7, [0x121A, 0x11B9, 0x80, 2, 0, 4, 0]))
    );
    assert_eq!(m.log.drained, 1);
}

#[test]
fn queue_follows_the_unit_table() {
    // The unit message before the add: dropped at receive.
    let mut m = with_monsters();
    m.recv(&[hex(STOP_1_6), hex(AC_1_6)].concat());
    assert_eq!(m.log.dropped.get(&0x6D), Some(&1));
    assert!(m.unit(M6).queue.is_empty());
    assert_eq!(m.drain(), 0);
    // Removed in the same receive: the queue goes with the unit.
    m.recv(&[hex(STOP_1_6), hex("0a 01 06 00 00 00")].concat());
    assert!(m.w.units.is_empty());
    assert_eq!(m.drain(), 0);
    // A message for a key that is not in the set addresses nothing,
    // even if a unit with that GUID has another type.
    let mut m = with_monsters();
    m.hex(AC_1_6);
    m.hex("0e 02 06 00 00 00 03 00 02 00 00 00");
    assert_eq!(m.log.dropped.get(&0x0E), Some(&1));
}

#[test]
fn adding_an_existing_key_replaces_the_unit() {
    let mut m = with_monsters();
    m.recv(&[hex(AC_1_6), hex(STOP_1_6)].concat());
    assert_eq!(m.unit(M6).queue.len(), 1);
    // The second add (another class) replaces the first: its queue is
    // dropped.
    m.hex("ac 06 00 00 00 05 00 1a 12 b9 11 80 0e 01");
    let u = m.unit(M6);
    assert_eq!((u.class, u.queue.len()), (5, 0));
    assert_eq!(m.w.units.len(), 1);
}

#[test]
fn drain_order() {
    let mut w = ClientWorld::default();
    for k in [
        UnitKey::new(MONSTER, 5),
        UnitKey::new(MONSTER, 0x85),
        UnitKey::new(MONSTER, 0x105),
        UnitKey::new(MONSTER, 6),
        UnitKey::new(OBJECT, 1),
        UnitKey::new(PLAYER, 9),
        UnitKey::new(MISSILE, 0x80),
        UnitKey::new(MISSILE, 0),
        UnitKey::new(5, 1),
    ] {
        w.units.insert(k, ClientUnit::new(k));
    }
    let guids: Vec<(u8, u32)> = update_order(&w.units)
        .into_iter()
        .map(|k| (k.unit_type, k.guid))
        .collect();
    assert_eq!(
        guids,
        [
            (MISSILE, 0x80),
            (MISSILE, 0),
            (PLAYER, 9),
            (MONSTER, 0x105),
            (MONSTER, 0x85),
            (MONSTER, 5),
            (MONSTER, 6),
            (OBJECT, 1),
        ]
    );
}

#[test]
fn pre_steps_and_one_byte_unit_ids_change_nothing() {
    let mut m = Model::default();
    let before = m.w.clone();
    // 0x6E–0x72: no addressed unit, no queue entry, no effect.
    m.recv(&hex("6e 6f 70 71 72"));
    assert_eq!(m.w, before);
    assert_eq!(m.log.dropped.values().sum::<u64>(), 5);
    // The largest unit-handler message (0x68, 21 bytes) fits a 26-byte
    // slot; D2RUST queues whole messages.
    assert_eq!(
        d2_proto::transport::server_message(0x68).unwrap().size,
        d2_proto::schema::SizeRule::Fixed(21)
    );
}

fn monster_at(w: &mut ClientWorld, mode: u32, x: u16, y: u16) -> UnitKey {
    let k = UnitKey::new(MONSTER, 3);
    let mut u = ClientUnit::new(k);
    u.mode = mode;
    u.position = Some((x, y));
    w.units.insert(k, u);
    k
}

fn hidden(_: &ClientUnit, _: i32, _: i32) -> bool {
    false
}

fn shown(_: &ClientUnit, _: i32, _: i32) -> bool {
    true
}

#[test]
fn position_check_vectors() {
    let none = ModelInputs::default();
    let shown = ModelInputs {
        visible: Some(VisibleFn::new(shown)),
        ..ModelInputs::default()
    };
    let hidden = ModelInputs {
        visible: Some(VisibleFn::new(hidden)),
        ..ModelInputs::default()
    };

    // The local player at (100, 100), mode 1: 4 > 3 → 0x5F with its own
    // position, not moved.
    let mut w = ClientWorld::default();
    let mut u = ClientUnit::new(P1);
    u.mode = 1;
    u.position = Some((100, 100));
    w.units.insert(P1, u);
    w.local_player = Some(P1);
    assert_eq!(
        check(&mut w, &none, P1, 104, 100, 0, 0, 0).unwrap(),
        Checked::Asked
    );
    assert_eq!(w.outgoing, vec![hex("5f 64 00 64 00")]);
    assert_eq!(w.units[&P1].position, Some((100, 100)));
    assert_eq!(w.units[&P1].server_point, (104, 100));

    // The same while the play preview predicts the walk: the
    // server's point is taken, nothing is sent; a placement since the
    // prediction was recorded (the model position moved) asks again.
    let mut w = ClientWorld::default();
    let mut u = ClientUnit::new(P1);
    u.mode = 1;
    u.position = Some((100, 100));
    w.units.insert(P1, u);
    w.local_player = Some(P1);
    w.set_local_walk(Some(((108 << 16) | 0x8000, (100 << 16) | 0x8000)), Some(1));
    assert_eq!(
        check(&mut w, &none, P1, 104, 100, 0, 0, 0).unwrap(),
        Checked::Followed
    );
    assert!(w.outgoing.is_empty());
    assert_eq!(w.units[&P1].position, Some((100, 100)));
    assert_eq!(w.units[&P1].server_point, (104, 100));
    w.units.get_mut(&P1).unwrap().position = Some((101, 100));
    assert_eq!(
        check(&mut w, &none, P1, 106, 100, 0, 0, 0).unwrap(),
        Checked::Asked
    );
    assert_eq!(w.outgoing, vec![hex("5f 65 00 64 00")]);

    // A prediction that disagrees with the model by more than a walk step is
    // stale (the server teleported the player).
    assert!(w.is_stale_prediction((0x8000, 0x8000)));
    assert!(!w.is_stale_prediction((101 << 16 | 0x8000, 100 << 16 | 0x8000)));

    // A monster, mode 1 (T 15): 10 ≤ 15 and y = cy (visible): nothing.
    let mut w = ClientWorld::default();
    let k = monster_at(&mut w, 1, 100, 100);
    assert_eq!(
        check(&mut w, &none, k, 110, 100, 0, 0, 0).unwrap(),
        Checked::Kept
    );
    assert_eq!(w.units[&k].server_point, (110, 100));
    assert_eq!(w.units[&k].position, Some((100, 100)));

    // Within tolerance, both points not visible: teleport.
    assert_eq!(
        check(&mut w, &hidden, k, 110, 105, 0, 0, 0).unwrap(),
        Checked::Moved
    );
    assert_eq!(w.units[&k].position, Some((110, 105)));
    // Visible: kept.
    let k = monster_at(&mut w, 1, 100, 100);
    assert_eq!(
        check(&mut w, &shown, k, 110, 105, 0, 0, 0).unwrap(),
        Checked::Kept
    );
    // No predicate yet: a loud error, not a guess.
    assert!(check(&mut w, &none, k, 110, 105, 0, 0, 0).is_err());

    // Mode 4 (T 5): 6 > 5 → teleport.
    let k = monster_at(&mut w, 4, 100, 100);
    assert_eq!(
        check(&mut w, &none, k, 106, 100, 0, 0, 0).unwrap(),
        Checked::Moved
    );
    assert_eq!(w.units[&k].position, Some((106, 100)));
    // Mode 7 (T 7): 7 is within.
    let k = monster_at(&mut w, 7, 100, 100);
    assert_eq!(
        check(&mut w, &none, k, 107, 100, 0, 0, 0).unwrap(),
        Checked::Kept
    );

    // Kind 0, tx > 0: d1 < 100, d2 < d1 → accepted, then the
    // visibility rule (visible: kept).
    let k = monster_at(&mut w, 4, 100, 100);
    // (x, y) = (106, 104): 6 > 5, d1 = 36 + 16 = 52; target (103, 103):
    // d2 = 9 + 9 = 18.
    assert_eq!(
        check(&mut w, &shown, k, 106, 104, 0, 103, 103).unwrap(),
        Checked::Kept
    );
    assert_eq!(w.units[&k].position, Some((100, 100)));
    // Accepted, then not visible: corrected.
    assert_eq!(
        check(&mut w, &hidden, k, 106, 104, 0, 103, 103).unwrap(),
        Checked::Moved
    );
    // d2 ≥ d1: corrected.
    let k = monster_at(&mut w, 4, 100, 100);
    assert_eq!(
        check(&mut w, &shown, k, 106, 104, 0, 110, 110).unwrap(),
        Checked::Moved
    );
    // d1 ≥ 100: corrected.
    let k = monster_at(&mut w, 4, 100, 100);
    assert_eq!(
        check(&mut w, &shown, k, 110, 100, 0, 101, 100).unwrap(),
        Checked::Moved
    );
    // Kind 1 (T 10) and kind 2 (T 0).
    let k = monster_at(&mut w, 4, 100, 100);
    assert_eq!(
        check(&mut w, &none, k, 110, 100, 1, 0, 0).unwrap(),
        Checked::Kept
    );
    assert_eq!(
        check(&mut w, &none, k, 101, 100, 2, 0, 0).unwrap(),
        Checked::Moved
    );

    // x = 0: nothing; a dead monster (mode 0xC): nothing.
    let k = monster_at(&mut w, 1, 100, 100);
    assert_eq!(
        check(&mut w, &none, k, 0, 150, 0, 0, 0).unwrap(),
        Checked::Skipped
    );
    assert_eq!(w.units[&k].server_point, (0, 0));
    let k = monster_at(&mut w, 0xC, 100, 100);
    assert_eq!(
        check(&mut w, &none, k, 150, 150, 0, 0, 0).unwrap(),
        Checked::Skipped
    );
    assert_eq!(w.units[&k].position, Some((100, 100)));
}

#[test]
fn creation_common_fields() {
    let mut m = with_monsters();
    // A player at (0, 0): not placed, seed {1, 666} stepped once.
    m.recv(&assign_player(0, 0));
    let u = m.unit(P1);
    assert_eq!((u.position, u.seed), (None, Some((0x6AC6_935F, 0))));
    // At another point the seed comes from the client room (not in the
    // model yet): unknown, not guessed.
    m.recv(&assign_player(0x1241, 0x11C4));
    assert_eq!(m.unit(P1).seed, None);
    assert_eq!(m.unit(P1).position, Some((0x1241, 0x11C4)));
    // A monster at (0, 0): {0, 666}.
    m.hex("ac 07 00 00 00 9a 00 00 00 00 00 80 0e 01");
    assert_eq!(m.unit(UnitKey::new(MONSTER, 7)).seed, Some((0, 666)));
}

const PET_SET: &str = "7a 01 07 4f 01 05 00 00 00 21 00 00 00";
const PET_REMOVE: &str = "7a 00 07 4f 01 05 00 00 00 21 00 00 00";

fn pet(pet_type: u8, pet: u32, owner: u32) -> PetRecord {
    PetRecord {
        class: 0x14F,
        pet_type,
        pet,
        owner,
        f1c: 100,
        gone: false,
        extra: None,
    }
}

#[test]
fn pet_action_sets_and_keeps_the_local_hireling() {
    let mut m = Model::default();
    // Empty list: −1.
    assert_eq!(m.w.hireling_guid(Some(UnitKey::new(PLAYER, 5))), u32::MAX);
    assert_eq!(m.w.hireling_guid(None), u32::MAX);
    m.hex(PET_SET);
    assert_eq!(m.w.pets, [pet(7, 0x21, 5)]);
    // The local player is GUID 5: the remove keeps the record, gone 1.
    m.w.local_player = Some(UnitKey::new(PLAYER, 5));
    m.hex(PET_REMOVE);
    assert_eq!(
        m.w.pets,
        [PetRecord {
            gone: true,
            ..pet(7, 0x21, 5)
        }]
    );
    assert_eq!(m.w.hireling_guid(m.w.local_player), 0x21);
    // Set again: the type-7 record is freed and a fresh one prepended.
    m.hex(PET_SET);
    assert_eq!(m.w.pets, [pet(7, 0x21, 5)]);
    assert!(m.log.rejected.is_empty());
}

#[test]
fn pet_action_list_order_update_and_remove() {
    let mut m = Model::default();
    // Type 3 pet 0x30 of owner 9, then type 7 pet 0x21 of owner 5.
    m.hex("7a 01 03 10 00 09 00 00 00 30 00 00 00").hex(PET_SET);
    assert_eq!(m.w.pets[0].pet, 0x21);
    assert_eq!(m.w.pets[1].pet, 0x30);
    // A set for an existing non-hireling GUID updates it in place.
    m.w.pets[1].f1c = 7;
    m.hex("7a 01 04 11 00 0a 00 00 00 30 00 00 00");
    assert_eq!(
        m.w.pets[1],
        PetRecord {
            class: 0x11,
            pet_type: 4,
            pet: 0x30,
            owner: 10,
            f1c: 7,
            gone: false,
            extra: None,
        }
    );
    // No local player: the hireling's remove frees it.
    m.hex(PET_REMOVE);
    assert_eq!(m.w.pets.len(), 1);
    // Remove of the other pet frees it; an unknown GUID: nothing.
    m.hex("7a 00 00 00 00 00 00 00 00 30 00 00 00")
        .hex("7a 00 00 00 00 00 00 00 00 99 00 00 00");
    assert!(m.w.pets.is_empty());
    assert!(m.log.rejected.is_empty());
}

#[test]
fn assign_merc_sets_with_extra_values() {
    let mut m = Model::default();
    m.hex("81 07 4f 01 05 00 00 00 21 00 00 00 aa bb cc dd 11 22 33 44");
    assert_eq!(
        m.w.pets,
        [PetRecord {
            extra: Some([0xDDCC_BBAA, 0x4433_2211, 0]),
            ..pet(7, 0x21, 5)
        }]
    );
    assert_eq!(m.w.hireling_guid(Some(UnitKey::new(PLAYER, 5))), 0x21);
    assert!(m.log.rejected.is_empty());
    // A recorded hire (MilHire seed 1234, frame 367):
    // seed and hire id land in +0x24 / +0x28, +0x2C := 0, +0x1C stays 100.
    let mut m = Model::default();
    m.hex("81 07 0f 01 01 00 00 00 0d 00 00 00 08 3b d9 51 53 0d 00 00");
    assert_eq!(
        m.w.pets,
        [PetRecord {
            class: 271,
            extra: Some([0x51D9_3B08, 0x0D53, 0]),
            ..pet(7, 13, 1)
        }]
    );
    assert!(m.log.rejected.is_empty());
}

#[test]
fn remove_unit_keeps_the_local_hireling() {
    let mut m = Model::default();
    let hireling = UnitKey::new(MONSTER, 0x21);
    m.put(hireling);
    m.put(P1);
    m.w.local_player = Some(P1);
    m.hex("7a 01 07 4f 01 01 00 00 00 21 00 00 00");
    m.hex("0a 01 21 00 00 00");
    assert!(m.w.units.contains_key(&hireling));
    // Another player's hireling is removed.
    m.w.pets[0].owner = 2;
    m.hex("0a 01 21 00 00 00");
    assert!(!m.w.units.contains_key(&hireling));
}

/// Level 1 (act 0, BlankScreen 1) and level 2 (act 0, BlankScreen 0);
/// level 40 is act 1.
fn levels() -> Vec<LevelRow> {
    let mut v = vec![LevelRow::default(); 41];
    v[1] = LevelRow {
        pal: 0,
        act: 0,
        blank_screen: true,
        sound_env: 0,
        draw_edges: false,
        rain: false,
        mud: false,
        critters: Default::default(),
        ambient: Default::default(),
    };
    (v[40].pal, v[40].act) = (1, 1);
    // A level whose `Pal` differs from its `Act` (as 125–127, 133–136).
    (v[3].pal, v[3].act) = (4, 0);
    v
}

fn room(x0: i32, y0: i32, w: i32, h: i32, level: u16) -> ActiveRoom {
    ActiveRoom {
        x0,
        y0,
        w,
        h,
        level,
        // No client DRLG in these tests: the id only names the room.
        room: d2_sim::drlg::DrlgRoomId(x0 as u32),
    }
}

#[test]
fn act_lookup_first_room_containing_the_point() {
    let rooms = [room(100, 200, 40, 40, 1), room(140, 200, 40, 40, 2)];
    assert_eq!(room_of_point(&rooms, 139, 239), Some(&rooms[0]));
    assert_eq!(room_of_point(&rooms, 140, 200), Some(&rooms[1]));
    assert_eq!(room_of_point(&rooms, 180, 200), None);
    assert_eq!(room_of_point(&rooms, 99, 200), None);
    // List order decides between overlapping rooms.
    let both = [room(0, 0, 10, 10, 3), room(0, 0, 10, 10, 4)];
    assert_eq!(room_of_point(&both, 5, 5).unwrap().level, 3);
}

#[test]
fn join_level_comes_from_the_room_of_the_0x15_placement() {
    use crate::world_view::{ModelFeed, NoFeed, ViewFeed};
    let mut m = Model::default();
    m.inputs.tables.levels = levels();
    let feed = ModelFeed {
        inner: NoFeed,
        levels: Some(levels()),
        ui_open_mode: None,
        map: None,
        preview: None,
        local_at: None,
        local_push: false,
        unit_tables: None,
        motion_offsets: Default::default(),
        motion_shadow_offsets: Default::default(),
        weather: None,
        client_seed: Default::default(),
    };
    m.recv(&assign_player(0, 0)).hex("0b 00 01 00 00 00");
    // 0x03 seq 142: palette act 0; u16@6 (town level 1) is not the level.
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    assert_eq!(m.w.palette_act, Some(0));
    // The level-1 room of origin tile (928, 904) brought in sight by 0x07
    // seq 144: sub-tiles from (4640, 4520); a 40 × 40 fixture rectangle.
    m.w.active_rooms = Some(vec![room(4640, 4520, 40, 40, 1)]);
    // Before 0x15: no room, no level, BlankScreen 0.
    assert_eq!(m.w.player_level(), None);
    assert!(!feed.blank_screen(&m.w).unwrap());
    // 0x15 seq 154: the player at (4673, 4548), level 1, BlankScreen 1.
    m.hex("15 00 01 00 00 00 41 12 c4 11 01");
    assert_eq!(m.unit(P1).position, Some((4673, 4548)));
    assert_eq!(m.w.player_level(), Some(1));
    assert!(feed.blank_screen(&m.w).unwrap());
    assert_eq!(m.w.palette_act, Some(0));
    // A non-zero point in no active room: fatal 0x168, not moved.
    m.hex("15 00 01 00 00 00 00 10 00 10 00");
    assert_eq!(m.rejected(), [(0x15, "fatal assert 0x168".to_owned())]);
    assert_eq!(m.unit(P1).position, Some((4673, 4548)));
    // A unit not in S: nothing, even at a point with no room.
    m.hex("15 00 09 00 00 00 00 10 00 10 00");
    assert_eq!(m.log.rejected.len(), 1);
}

#[test]
fn level_is_the_rooms_not_the_town_of_0x03() {
    let mut m = Model::default();
    m.inputs.tables.levels = levels();
    m.recv(&assign_player(0, 0)).hex("0b 00 01 00 00 00");
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.w.active_rooms = Some(vec![room(100, 100, 40, 40, 2)]);
    m.hex("15 00 01 00 00 00 6e 00 6e 00 00");
    assert_eq!(m.w.act.unwrap().town_level, 1);
    assert_eq!(m.w.player_level(), Some(2));
}

#[test]
fn room_change_to_another_act_switches_the_palette() {
    let mut m = Model::default();
    m.inputs.tables.levels = levels();
    m.recv(&assign_player(0, 0)).hex("0b 00 01 00 00 00");
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.w.active_rooms = Some(vec![
        room(100, 100, 40, 40, 1),
        room(140, 100, 40, 40, 2),
        room(300, 300, 40, 40, 40),
    ]);
    // First placement (no old room) in a level whose `Pal` is not the
    // palette of the act loaded: its `Pal`.
    m.hex("15 00 01 00 00 00 2c 01 2c 01 00");
    assert_eq!(m.w.palette_act, Some(1));
    // Act 1 → act 0 (level 40 → 1): switch.
    m.w.palette_act = Some(9);
    m.hex("15 00 01 00 00 00 6e 00 6e 00 00");
    assert_eq!(m.w.palette_act, Some(0));
    m.w.palette_act = Some(3);
    // Level 1 → 2, same act: no switch.
    m.hex("15 00 01 00 00 00 96 00 6e 00 00");
    assert_eq!(m.w.palette_act, Some(3));
    // Level 2 → 40: act 1.
    m.hex("15 00 01 00 00 00 2c 01 2c 01 00");
    assert_eq!(m.w.palette_act, Some(1));
    assert!(m.log.rejected.is_empty());
}

/// A warp across acts (the dev tools' `warp`): 0x03 loads the act and its
/// palette, then the first placement lands in a level whose `Pal` differs
/// from the act (as 133-136): the level's `Pal` is shown, not the act's.
/// A first placement in a level of the act's own palette changes nothing.
#[test]
fn first_placement_after_an_act_load_shows_the_levels_pal() {
    let mut m = Model::default();
    m.inputs.tables.levels = levels();
    m.recv(&assign_player(0, 0)).hex("0b 00 01 00 00 00");
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.w.active_rooms = Some(vec![room(100, 100, 40, 40, 1)]);
    m.hex("15 00 01 00 00 00 6e 00 6e 00 00");
    assert_eq!(m.w.palette_act, Some(0));
    // Act 0 loaded again (its rooms freed), the player placed in level 3
    // (`Act` 0, `Pal` 4) with no old room.
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    assert_eq!(m.w.palette_act, Some(0));
    m.w.active_rooms = Some(vec![room(140, 100, 40, 40, 3)]);
    assert_eq!(m.w.local_room(), None);
    m.hex("15 00 01 00 00 00 96 00 6e 00 00");
    assert_eq!(m.w.palette_act, Some(4));
    assert!(m.log.rejected.is_empty());
}

#[test]
fn room_change_reads_levels_pal_not_act() {
    let mut m = Model::default();
    m.inputs.tables.levels = levels();
    m.recv(&assign_player(0, 0)).hex("0b 00 01 00 00 00");
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.w.active_rooms = Some(vec![
        room(100, 100, 40, 40, 1),
        room(140, 100, 40, 40, 3),
        room(300, 300, 40, 40, 40),
    ]);
    m.hex("15 00 01 00 00 00 6e 00 6e 00 00");
    assert_eq!(m.w.palette_act, Some(0));
    // Level 1 → 3: the same `Act` 0, but `Pal` 4: the palette switches.
    m.hex("15 00 01 00 00 00 96 00 6e 00 00");
    assert_eq!(m.w.palette_act, Some(4));
    // Level 3 (`Act` 0, `Pal` 4) → 40 (`Act` 1, `Pal` 1): to `Pal` 1.
    m.hex("15 00 01 00 00 00 2c 01 2c 01 00");
    assert_eq!(m.w.palette_act, Some(1));
    assert!(m.log.rejected.is_empty());
}

#[test]
fn join_refused_maps_the_code_and_writes_nothing() {
    use super::super::output::Output;
    use super::session::join_refused_error;
    // The jump table, every code.
    let want: [u8; 28] = [
        9, 0, 1, 2, 3, 4, 5, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 9, 0x19,
        0x1A, 0x1C, 0x1B, 9,
    ];
    for (c, n) in want.iter().enumerate() {
        assert_eq!(join_refused_error(c as u32), *n, "code {c}");
    }
    assert_eq!(join_refused_error(u32::MAX), 9);
    // The single-player codes.
    let sp: Vec<u8> = [0x13, 0x14, 0x15, 0x17, 0x18]
        .map(join_refused_error)
        .to_vec();
    assert_eq!(sp, [0x16, 0x17, 0x18, 0x19, 0x1A]);
    let mut m = Model::default();
    m.w.in_game = true;
    m.hex("b4 13 00 00 00");
    assert!(m.log.rejected.is_empty());
    assert_eq!(m.out, [Output::JoinRefused { error: 0x16 }]);
    assert!(m.w.in_game && !m.w.exit_requested);
}

#[test]
fn room_freed_unit_sends_0x4b_once() {
    let mut m = Model::default();
    let k = UnitKey::new(MONSTER, 0x105);
    let u = m.put(k);
    u.room_freed = true;
    u.flag_ex = 0x20;
    u.queue.push(hex(STOP_1_6));
    m.drain();
    assert_eq!(m.w.outgoing, [hex("4b 01 00 00 00 05 01 00 00")]);
    let u = m.unit(k);
    assert!(!u.room_freed && u.flag_ex == 0);
    // The queue was not drained that pass.
    assert_eq!(u.queue.len(), 1);
}
