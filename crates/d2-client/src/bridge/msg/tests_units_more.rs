//! More test vectors of the unit messages: "A" and "B" are two recorded
//! packet traces; the rest synthetic.

use super::super::output::{Output, ShrineFxKind};
use super::super::world::{
    KindData, MonsterData, ObjectData, ObjectRow, PlayerData, RosterRecord, StatSend, StateRow,
    UnitKey, MONSTER, OBJECT, PLAYER,
};
use super::support::{hex, Model};

const P1: UnitKey = UnitKey::new(PLAYER, 1);

/// A stat table where stat 172 has `Send Bits` 2 (the recorded 0xAA).
fn with_stats() -> Model {
    let mut m = Model::default();
    m.inputs.tables.stats = vec![StatSend::default(); 360];
    m.inputs.tables.stats[172].bits = 2;
    m
}

#[test]
fn add_unit_states_b103_a184() {
    let mut m = with_stats();
    m.put(P1);
    m.hex("aa 00 01 00 00 00 0c 69 59 f9 ff 1f");
    let u = m.unit(P1);
    assert!(u.states.contains(&105));
    assert_eq!(u.state_lists[&105].get(&(172, 0)), Some(&2));
    let k = UnitKey::new(MONSTER, 8);
    m.put(k);
    m.hex("aa 01 08 00 00 00 0c 69 59 f9 ff 1f");
    assert!(m.unit(k).states.contains(&105));
    assert!(m.log.rejected.is_empty());
    // A unit not in S: nothing.
    m.hex("aa 01 09 00 00 00 0c 69 59 f9 ff 1f");
    assert!(!m.w.units.contains_key(&UnitKey::new(MONSTER, 9)));
}

#[test]
fn add_unit_states_abort_and_overflow() {
    let mut m = with_stats();
    m.put(P1);
    // State 5, list bit 1, stat id 1 (`Send Bits` 0): state on, no stat,
    // the rest ignored: bits 05, then 1, then id 1 (9 bits).
    m.hex("aa 00 01 00 00 00 0a 05 03 00");
    let u = m.unit(P1);
    assert!(u.states.contains(&5) && u.state_lists.is_empty());
    assert!(m.log.rejected.is_empty());
    // No closing 0xFF: the states before are applied, then an error.
    m.hex("aa 00 01 00 00 00 08 07");
    assert!(m.unit(P1).states.contains(&7));
    assert_eq!(m.rejected().len(), 1);
}

#[test]
fn set_delayed_and_end_state() {
    let mut m = with_stats();
    m.put(P1);
    // 0xA8: size 10, state 9, then the stream: stat 172 (9 bits: ac and
    // bit 0 of fc), value 2 (2 bits), then a stat id cut by the end
    // (31: `Send Bits` 0 ends the stream).
    m.hex("a8 00 01 00 00 00 0a 09 ac fc");
    assert!(m.unit(P1).states.contains(&9));
    assert_eq!(m.unit(P1).state_lists[&9].get(&(172, 0)), Some(&2));
    // 0xA7 on a state with a list and no states row: the keep flag is
    // needed, so the handler refuses (the tables are an input).
    m.hex("a7 00 01 00 00 00 09");
    assert_eq!(m.rejected().len(), 1);
    m.inputs.tables.states = vec![StateRow::default(); 200];
    m.hex("a7 00 01 00 00 00 09");
    assert!(!m.unit(P1).state_lists.contains_key(&9), "freed");
    m.hex("a9 00 01 00 00 00 09");
    assert!(!m.unit(P1).states.contains(&9));
    // A dead player (mode 17) with the dead-bit-only flag: the bit only.
    m.inputs.tables.states[3].dead_bit_only = true;
    m.w.units.get_mut(&P1).unwrap().mode = 17;
    m.w.units
        .get_mut(&P1)
        .unwrap()
        .state_lists
        .insert(3, [((1, 0), 1)].into());
    m.hex("a7 00 01 00 00 00 03");
    let u = m.unit(P1);
    assert!(u.states.contains(&3) && u.state_lists.contains_key(&3));
}

#[test]
fn assign_level_warp_creates_a_unit() {
    let mut m = Model::default();
    m.hex("09 05 0a 00 00 00 03 10 00 20 00");
    let u = m.unit(UnitKey::new(5, 10));
    assert_eq!((u.class, u.position), (3, Some((0x10, 0x20))));
}

#[test]
fn unit_overlay_sounds() {
    let mut m = Model::default();
    m.inputs.tables.overlay_count = 200;
    let k = UnitKey::new(MONSTER, 4);
    m.put(k);
    m.hex("11 01 04 00 00 00 97 00")
        .hex("11 01 04 00 00 00 05 00");
    // Overlay past the count, or an absent unit: nothing.
    m.hex("11 01 04 00 00 00 c8 00")
        .hex("11 01 05 00 00 00 05 00");
    assert_eq!(
        m.out,
        [
            Output::UnitOverlay {
                unit: k,
                overlay: 151,
                mode: 2,
                sound: 396
            },
            Output::UnitOverlay {
                unit: k,
                overlay: 5,
                mode: 2,
                sound: 0
            },
        ]
    );
}

#[test]
fn npc_enchants_write_monster_data() {
    let mut m = Model::default();
    let k = UnitKey::new(MONSTER, 4);
    m.put(k).kind = KindData::Monster(Box::default());
    m.hex("57 04 00 00 00 01 22 11 05 06 07 00 01 00");
    let KindData::Monster(d) = &m.unit(k).kind else {
        panic!()
    };
    assert_eq!(
        **d,
        MonsterData {
            name_seed: 0x1122,
            flags: 4 | 8,
            umods: [5, 6, 7, 0, 0, 0, 0, 0, 0],
            ..MonsterData::default()
        }
    );
    assert_eq!(
        m.out,
        [Output::UmodFx {
            unit: k,
            umods: [5, 6, 7, 0, 0, 0, 0, 0, 0],
            flag8: true
        }]
    );
    // u8@5 ≠ 1: nothing.
    m.out.clear();
    m.hex("57 04 00 00 00 02 22 11 05 06 07 00 01 00");
    assert!(m.out.is_empty());
}

#[test]
fn portal_flags_and_town_portal_state() {
    let mut m = Model::default();
    m.hex("5f 01 00 00 00");
    assert_eq!(m.rejected(), [(0x5F, "fatal assert 0xD15".to_string())]);
    m.put(P1).kind = KindData::Player(PlayerData::default());
    m.w.local_player = Some(P1);
    m.hex("5f 01 00 00 00");
    let KindData::Player(p) = &m.unit(P1).kind else {
        panic!()
    };
    assert_eq!(p.f2c, 1);
    let o = UnitKey::new(OBJECT, 9);
    let u = m.put(o);
    u.class = 59;
    u.kind = KindData::Object(ObjectData::default());
    m.hex("60 03 01 09 00 00 00").hex("60 00 04 09 00 00 00");
    let KindData::Object(d) = &m.unit(o).kind else {
        panic!()
    };
    assert_eq!((d.interact, d.portal_flags), (4, 3), "flags only OR");
    m.w.units.get_mut(&o).unwrap().class = 2;
    m.hex("60 03 01 09 00 00 00");
    assert_eq!(m.rejected()[1], (0x60, "fatal assert 0x560".to_string()));
}

#[test]
fn effect_outputs() {
    let mut m = Model::default();
    m.put(P1);
    m.w.local_player = Some(P1);
    m.inputs.tables.monsters = vec![None; 10];
    let mut missile = hex("73 00 00 00 00 05 00");
    missile.resize(0x19, 0);
    missile.extend(hex("01 08 00 00 00 02 03"));
    m.recv(&missile);
    m.hex("7e 00 00 00 ff");
    m.hex("a4 09 00").hex("a4 0a 00");
    assert_eq!(
        m.out,
        [
            Output::ClientMissile {
                owner: Some(P1),
                class: 5,
                f07: 0,
                f0b: 0,
                f0f: 0,
                f13: 0,
                f17: 0,
                source: UnitKey::new(MONSTER, 8),
                f1e: 2,
                f1f: 3,
            },
            Output::CommonCof { act: 0 },
            Output::MonsterPreload { class: 9 },
        ]
    );
}

#[test]
fn player_corpse_assign() {
    let mut m = Model::default();
    let p = m.put(P1);
    p.mode = 5;
    p.position = Some((10, 20));
    m.w.local_player = Some(P1);
    let k = UnitKey::new(PLAYER, 7);
    m.put(k);
    m.hex("74 01 01 00 00 00 07 00 00 00");
    assert_eq!(m.unit(P1).mode, 0);
    assert_eq!(m.unit(k).position, Some((10, 20)));
    // u8@1 = 0: nothing.
    m.w.units.get_mut(&P1).unwrap().mode = 5;
    m.hex("74 00 01 00 00 00 07 00 00 00");
    assert_eq!(m.unit(P1).mode, 5);
}

#[test]
fn monster_f40_and_npc_heal() {
    let mut m = Model::default();
    let k = UnitKey::new(MONSTER, 0x13);
    m.put(k).kind = KindData::Monster(Box::default());
    m.hex("98 13 00 00 00 ff ff");
    let KindData::Monster(d) = &m.unit(k).kind else {
        panic!()
    };
    assert_eq!(d.f40, Some(-1));
    m.hex("ab 01 13 00 00 00 30");
    assert_eq!(m.unit(k).stat(6), 0x3000);
    // A player: the roster life percent.
    m.put(P1);
    m.w.roster.push(RosterRecord {
        guid: 1,
        ..RosterRecord::default()
    });
    m.hex("ab 00 01 00 00 00 40");
    assert_eq!(m.w.roster[0].life, 50);
    // 0x0D type 0 also sets it.
    m.hex("0d 00 01 00 00 00 01 20 13 84 13 00 21");
    m.drain();
    assert_eq!(m.w.roster[0].life, 0x21);
}

fn joined_a267() -> Vec<u8> {
    let mut b = hex("5b 24 00 01 00 00 00 04");
    let mut name = b"charactertest".to_vec();
    name.resize(16, 0);
    b.extend(name);
    b.extend(hex("01 00 ff ff 00 00 00 00 00 00 00 00"));
    b
}

#[test]
fn roster_a267_a222604() {
    let mut m = Model::default();
    m.recv(&joined_a267());
    m.hex("65 01 00 00 00 01 00");
    let mut name = [0u8; 16];
    name[..13].copy_from_slice(b"charactertest");
    let want = RosterRecord {
        name,
        guid: 1,
        class: 4,
        f20: 1,
        f22: 0xFFFF,
        kills: 1,
        strings: vec![0, 0, 0, 0, 0],
        ..RosterRecord::default()
    };
    assert_eq!(m.w.roster, std::slice::from_ref(&want));
    assert_eq!(m.out.len(), 2);
    assert_eq!(m.out[1], Output::RosterChanged { roster: vec![want] });
    // A kill count of 0x8001 is sign-extended.
    m.hex("65 01 00 00 00 01 80");
    assert_eq!(m.w.roster[0].kills, -32767);
    // An unknown GUID: nothing.
    m.hex("65 02 00 00 00 01 00");
    assert_eq!(m.out.len(), 3);
}

#[test]
fn corpses_portals_and_leaving() {
    let mut m = Model::default();
    // 0x8E for an unknown player: a new inactive record.
    m.hex("8e 01 05 00 00 00 07 00 00 00");
    assert_eq!(m.w.roster_inactive[0].corpses, [7]);
    m.recv(&joined_a267());
    m.hex("8e 01 01 00 00 00 07 00 00 00");
    // 0x65 for GUID 7 finds the record of player 1 (corpse-list match).
    m.hex("65 07 00 00 00 03 00");
    assert_eq!(m.w.roster[0].kills, 3);
    // 0x82: the portals' owner names and the roster's portal GUIDs.
    let o = UnitKey::new(OBJECT, 0x20);
    m.put(o).kind = KindData::Object(ObjectData::default());
    let mut portal = hex("82 01 00 00 00 61 62");
    portal.resize(21, 0);
    portal.extend(hex("20 00 00 00 21 00 00 00"));
    m.recv(&portal);
    assert_eq!(m.w.roster[0].portals, (0x20, 0x21));
    let KindData::Object(d) = &m.unit(o).kind else {
        panic!()
    };
    assert_eq!(&d.owner_name.unwrap()[..3], b"ab\0");
    // 0x8E remove, then 0x5C.
    m.hex("8e 00 01 00 00 00 07 00 00 00");
    assert!(m.w.roster[0].corpses.is_empty());
    m.out.clear();
    m.hex("5c 01 00 00 00");
    assert!(m.w.roster.is_empty());
    assert_eq!(m.out, [Output::RosterChanged { roster: vec![] }]);
}

/// A shrine object (2, 9) of class 3 (`SubClass` bit 0) with `Code` code.
fn shrine(code: Option<u8>) -> Model {
    let mut m = Model::default();
    m.inputs.tables.objects = vec![ObjectRow::default(); 4];
    m.inputs.tables.objects[3].subclass = 1;
    let u = m.put(UnitKey::new(OBJECT, 9));
    u.class = 3;
    u.kind = KindData::Object(ObjectData {
        shrine: code,
        ..ObjectData::default()
    });
    m
}

#[test]
fn shrine_on_use_sound() {
    let mut m = shrine(Some(1));
    m.put(P1);
    m.hex("4d 02 09 00 00 00 01 00 00 00 01 00 00 00 00 00 00");
    m.drain();
    let o = UnitKey::new(OBJECT, 9);
    assert_eq!(m.unit(o).last_mode_request.map(|r| r.code), Some(0x15));
    assert_eq!(
        m.out,
        [Output::ShrineSound {
            sound: 0xA71,
            player: P1
        }]
    );
    // The operator GUID 7 is unknown: no output.
    m.out.clear();
    m.hex("4d 02 09 00 00 00 07 00 00 00 01 00 00 00 00 00 00");
    m.drain();
    assert!(m.out.is_empty());
    // An on-use shrine (code 16): the function, then the sound.
    let mut m = shrine(Some(16));
    m.put(P1);
    m.hex("4d 02 09 00 00 00 01 00 00 00 01 00 00 00 00 00 00");
    m.drain();
    assert_eq!(
        m.out,
        [
            Output::ShrineFx {
                kind: ShrineFxKind::OnUse,
                code: 16,
                object: o,
                player: Some(P1),
                overlays: [-1, -1],
                mode: 0,
            },
            Output::ShrineSound {
                sound: 0xA76,
                player: P1
            }
        ]
    );
    // A non-shrine object whose operator is known: fatal 0x34D.
    let mut m = shrine(None);
    m.inputs.tables.objects[3].subclass = 0;
    m.put(P1);
    m.hex("4d 02 09 00 00 00 01 00 00 00 01 00 00 00 00 00 00");
    m.drain();
    assert_eq!(m.rejected(), [(0x4D, "fatal assert 0x34D".to_string())]);
}

#[test]
fn shrine_on_mode_overlays() {
    let mut m = shrine(Some(6));
    m.hex("0e 02 09 00 00 00 03 00 01 00 00 00");
    m.drain();
    // The object mode change (model) sets mode 1 and makes its mode
    // sound call inside the change, before
    // the shrine's on-mode function.
    assert_eq!(
        m.out[0],
        Output::ObjectSound(super::super::objects::ObjSound::Mode {
            unit: super::super::objects::ObjUnit {
                key: UnitKey::new(OBJECT, 9),
                client_only: false,
            },
            class: 3,
            mode: 1,
            local_dist: super::super::objects::NO_LOCAL_DISTANCE,
        })
    );
    m.out.remove(0);
    assert_eq!(
        m.out,
        [Output::ShrineFx {
            kind: ShrineFxKind::OnMode,
            code: 6,
            object: UnitKey::new(OBJECT, 9),
            player: None,
            overlays: [0x3B, 0x39],
            mode: 1,
        }]
    );
    // A shrine without shrine data: fatal 0x37A.
    let mut m = shrine(None);
    m.hex("0e 02 09 00 00 00 03 00 01 00 00 00");
    m.drain();
    assert_eq!(m.rejected(), [(0x0E, "fatal assert 0x37A".to_string())]);
}

#[test]
fn stat_172_changes_the_rooms_allied_count() {
    use super::super::drlg::DrlgRoomId;
    let mut m = with_stats();
    m.inputs.tables.states = vec![StateRow::default(); 200];
    // State on keeps the existing list, so the second 0xAA reads old 2.
    m.inputs.tables.states[105].keep_list = true;
    let k = UnitKey::new(MONSTER, 8);
    m.put(k);
    m.w.room_units.place(k, Some(DrlgRoomId(3)));
    // B 103's 0xAA: state 105 with stat 172 = 2 on a new list (old 4).
    m.hex("aa 01 08 00 00 00 0c 69 59 f9 ff 1f");
    assert_eq!(m.w.room_allied.get(&DrlgRoomId(3)), Some(&1));
    // The same value again: old = new, no change.
    m.hex("aa 01 08 00 00 00 0c 69 59 f9 ff 1f");
    assert_eq!(m.w.room_allied.get(&DrlgRoomId(3)), Some(&1));
    assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
}
