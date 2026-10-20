//! Test vectors of the PC 1 breadth pass: 0x75, 0x8F, 0xAF / 0xB0, 0xB2
//! and the shared no-op ids.

use super::super::check::latency_for_test;
use super::super::output::Output;
use super::super::world::{PetRecord, RosterRecord, UnitKey, MONSTER, PLAYER};
use super::support::{hex, Model};

#[test]
fn party_info_updates_roster_and_emits() {
    let mut m = Model::default();
    m.w.roster.push(RosterRecord {
        guid: 1,
        ..RosterRecord::default()
    });
    // Recording B: GUID 1, party 0xFFFF, level 2, u16@11 = 1.
    m.hex("75 01 00 00 00 ff ff 02 00 00 00 01 00");
    let r = &m.w.roster[0];
    assert_eq!((r.f22, r.f20, r.f30), (0xFFFF, 2, 1));
    assert_eq!(m.out.len(), 1);
    assert!(matches!(m.out[0], Output::RosterChanged { .. }));
    assert!(m.log.rejected.is_empty());
}

#[test]
fn party_info_unknown_guid_and_corpse_holder_do_nothing() {
    let mut m = Model::default();
    m.hex("75 09 00 00 00 01 00 02 00 00 00 01 00");
    assert!(m.w.roster.is_empty() && m.out.is_empty());
    m.w.roster.push(RosterRecord {
        guid: 1,
        corpses: vec![7],
        ..RosterRecord::default()
    });
    m.hex("75 07 00 00 00 01 00 02 00 00 00 01 00");
    assert_eq!(m.w.roster[0].f22, 0);
    assert!(m.out.is_empty());
}

#[test]
fn party_info_pet_pass_sets_palette() {
    let mut m = Model::default();
    m.w.local_player = Some(UnitKey::new(PLAYER, 1));
    m.w.roster.push(RosterRecord {
        guid: 1,
        ..RosterRecord::default()
    });
    let pet = |pet, owner, pet_type| PetRecord {
        class: 0,
        pet_type,
        pet,
        owner,
        f1c: 100,
        gone: false,
        extra: None,
    };
    m.w.pets = vec![pet(20, 1, 4), pet(21, 2, 4), pet(22, 2, 4), pet(23, 2, 7)];
    m.put(UnitKey::new(MONSTER, 20));
    m.put(UnitKey::new(MONSTER, 21));
    m.put(UnitKey::new(MONSTER, 23));
    m.hex("75 01 00 00 00 01 00 02 00 00 00 01 00");
    assert_eq!(
        m.w.pet_palette
            .iter()
            .map(|(k, v)| (k.guid, *v))
            .collect::<Vec<_>>(),
        [(20, 1), (21, 0)]
    );

    // The owner has a roster record in the local player's party
    // (+0x22 equal and not 0xFFFF): t = 1; another party: t = 0.
    m.w.pet_palette.clear();
    for (guid, party) in [(2u32, 5u16), (3, 6)] {
        m.w.roster.push(RosterRecord {
            guid,
            f22: party,
            ..RosterRecord::default()
        });
    }
    m.w.pets[1].owner = 3;
    m.w.roster[0].f22 = 5;
    m.hex("75 01 00 00 00 05 00 02 00 00 00 01 00");
    assert_eq!(m.w.pet_palette[&UnitKey::new(MONSTER, 21)], 0);
    m.w.pets[1].owner = 2;
    m.hex("75 01 00 00 00 05 00 02 00 00 00 01 00");
    assert_eq!(m.w.pet_palette[&UnitKey::new(MONSTER, 21)], 1);

    // No local player: every record reads 0; type 5 is not written.
    m.w.pet_palette.clear();
    m.w.local_player = None;
    m.w.pets.push(pet(24, 1, 5));
    m.put(UnitKey::new(MONSTER, 24));
    m.hex("75 01 00 00 00 05 00 02 00 00 00 01 00");
    assert_eq!(
        m.w.pet_palette
            .iter()
            .map(|(k, v)| (k.guid, *v))
            .collect::<Vec<_>>(),
        [(20, 0), (21, 0)]
    );
}

#[test]
fn connection_flag() {
    let mut m = Model::default();
    assert!(!m.w.connected);
    m.hex("af 00");
    assert!(m.w.connected);
    m.hex("b0");
    assert!(!m.w.connected);
    assert!(m.log.unowned.is_empty() && m.log.rejected.is_empty());
}

#[test]
fn pong_single_player_zero() {
    let mut m = Model::default();
    let mut b = vec![0x8F];
    b.resize(33, 0);
    m.recv(&b);
    assert_eq!(m.w.ping.rtt, 0);
    assert_eq!(m.w.ping.samples, 1);
    assert_eq!(latency_for_test(&m.w), 0);
    // The tolerance reads the model field: 78 ms gives L = 1.
    m.w.ping.rtt = 78;
    assert_eq!(latency_for_test(&m.w), 1);
    m.w.ping.rtt = 77;
    assert_eq!(latency_for_test(&m.w), 0);
}

#[test]
fn no_effect_ids_change_nothing() {
    for (id, n) in [
        (0x79u8, 6usize),
        (0x7F, 0),
        (0x8B, 6),
        (0x8C, 0),
        (0x8D, 0),
        (0x90, 0),
        (0xAE, 0),
        (0xB2, 0),
        (0xB3, 0),
    ] {
        let Some(m) = d2_proto::transport::server_message(id) else {
            panic!("{id}")
        };
        let size = m.size.fixed().unwrap_or(0);
        if size == 0 {
            continue;
        }
        let mut model = Model::default();
        let mut b = vec![id];
        b.resize(size.max(n), 0);
        model.recv(&b);
        assert!(model.log.unowned.is_empty(), "{id:#x}");
        assert!(model.log.rejected.is_empty(), "{id:#x}");
        assert_eq!(model.w, Default::default(), "{id:#x}");
    }
    let _ = hex("00");
}
