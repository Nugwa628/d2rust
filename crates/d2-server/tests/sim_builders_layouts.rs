//! The d2-sim S→C builders against the layouts of `tables/sim/server-messages.tsv`
//! (the layout column, generated into `d2_proto::server`): each built message,
//! decoded with its generated type and encoded again, gives the same bytes. A
//! byte the builder writes outside the layout, or a field the layout places
//! elsewhere, fails here. Runs in the default test tier (no game files).

use d2_proto::server as gen;
use d2_proto::FixedMessage;
use d2_sim::monsters::mode_message::{self as mm, Move, MoveToUnit};
use d2_sim::path::walk::messages as walk;
use d2_sim::units::messages as unit;
use d2_sim::world::hirelings::pets;
use d2_sim::world::objects;

/// `b` round-trips through `M`'s layout.
fn keeps<M: FixedMessage + std::fmt::Debug>(b: &[u8]) {
    let m = M::decode(b).unwrap_or_else(|e| panic!("0x{:02X}: {e}", b[0]));
    let mut out = vec![0; M::SIZE];
    m.write(&mut out);
    assert_eq!(out, b, "0x{:02X}: {m:?}", b[0]);
}

// Unit message builders round-trip through their layouts.
#[test]
fn unit_builders_follow_the_layouts() {
    keeps::<gen::RemoveUnit>(&unit::remove_unit(1, 0x1234_5678));
    keeps::<gen::MapHide>(&unit::map_hide(0x1122, 0x3344, 0x55));
    keeps::<gen::AssignObject>(&unit::assign_object(
        0x0102_0304,
        0x177,
        0x1A2B,
        0x3C4D,
        2,
        1,
    ));
    keeps::<gen::AssignLevelWarp>(&unit::assign_warp(5, 0x0A0B_0C0D, 0x1F, 0x1111, 0x2222));
    keeps::<gen::SetSkill>(&unit::set_skill(0, 1, 1, 0x0123, 0xDEAD_BEEF));
    keeps::<gen::AssignHotkey>(&unit::assign_hotkey(3, 0x0FFF, true, 0xCAFE_F00D));
    keeps::<gen::PortalFlags>(&unit::portal_flags(0x8765_4321));
    keeps::<gen::ReassignPlayer>(&walk::reassign_player(0, 1, 0x131D, 0x1381, 1));
    keeps::<gen::ObjectState>(&objects::state_message(0x77, true, 0x0102_0304));
}

// Monster message builders round-trip through their layouts.
#[test]
fn monster_builders_follow_the_layouts() {
    keeps::<gen::MonsterMove>(&mm::monster_move(Move {
        guid: 0x0102_0304,
        code: 2,
        x: 0x1A2B,
        y: 0x3C4D,
        s: 7,
        t: 13,
        velocity: 0x0600,
        max_distance: 9,
    }));
    keeps::<gen::MonsterMoveToTarget>(&mm::move_to_unit(MoveToUnit {
        guid: 0x0102_0304,
        code: 2,
        x: 0x1A2B,
        y: 0x3C4D,
        a: 1,
        b: 0x0506_0708,
        s: 7,
        t: 13,
        velocity: 0x0600,
        max_distance: 9,
    }));
    keeps::<gen::MonsterState>(&mm::monster_state(0x0102_0304, 3, 0x1111, 0x2222, 4, 5));
    keeps::<gen::MonsterAttack>(&mm::monster_attack(
        0x0102_0304,
        4,
        1,
        0x0506_0708,
        6,
        0x1A2B,
        0x3C4D,
    ));
    keeps::<gen::MonsterStop>(&mm::monster_stop(0x0102_0304, 0x1A2B, 0x3C4D, 0x80));
}

// Pet message builders round-trip through their layouts.
#[test]
fn pet_builders_follow_the_layouts() {
    keeps::<gen::AssignMerc>(&pets::assign_merc(
        0x0150,
        1,
        0x0A0B_0C0D,
        0x1234_5678,
        0x0F0E,
    ));
    keeps::<gen::PetAction>(&pets::pet_action(1, 7, 0x0150, 0x0A0B_0C0D, 1));
}

// M08: a byte outside the layout (0x73's bytes 1–4 are unlisted) and a
// field moved by the layout are both reported.
#[test]
fn the_check_fails_on_an_unlisted_byte_or_a_moved_field() {
    let mut b = vec![0u8; 32];
    b[0] = 0x73;
    b[1] = 1;
    assert!(std::panic::catch_unwind(|| keeps::<gen::Unknown73>(&b)).is_err());
    b[1] = 0;
    keeps::<gen::Unknown73>(&b);
    // 0x7A read with 0x81's layout: the id check refuses it.
    let pet = pets::pet_action(1, 7, 0x0150, 0x0A0B_0C0D, 1);
    assert!(std::panic::catch_unwind(|| keeps::<gen::AssignMerc>(&pet)).is_err());
}
