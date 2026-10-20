use super::*;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn send(i: &ModeInput) -> Vec<u8> {
    match mode_message(i) {
        ModeMessage::Send(b) => b,
        ModeMessage::Stop(b) => b.to_vec(),
        other => panic!("no bytes: {other:?}"),
    }
}

/// The measured death pair (frames 2724 and
/// 2748): code 8 at the path target with d = the path direction and
/// e = unit +0xB0; code 9 at the unit's cell with e = 0. The vector's
/// "cell (4757, 5461)" is the (a, b) of mode 0, which step 4 takes from
/// the path target ("target from path"); the cell one step away is the
/// mode-12 vector's.
#[test]
fn death_pair_vectors() {
    let mut i = ModeInput {
        mode: mode::DT,
        guid: 0x13,
        cell: (4756, 5461),
        path_target: (4757, 5461),
        direction: 0x38,
        unit_b0: 6,
        ..ModeInput::default()
    };
    assert_eq!(send(&i), hex("69 13000000 08 9512 5515 38 06"));
    i.mode = mode::DD;
    assert_eq!(send(&i), hex("69 13000000 09 9412 5515 38 00"));
    // A path that never had a target (measured frame 2882).
    let j = ModeInput {
        mode: mode::DT,
        guid: 0x1B,
        cell: (4000, 4000),
        direction: 0x38,
        unit_b0: 6,
        ..ModeInput::default()
    };
    assert_eq!(send(&j), hex("69 1b000000 08 0000 0000 38 06"));
}

/// Mode 1: 0x6D with the unit's cell and the life fraction (the caller
/// adds 1 to stat 328); mode 6 without a target: (a, b) = (0, −1).
#[test]
fn stop_and_block_vectors() {
    let i = ModeInput {
        mode: mode::NU,
        guid: 6,
        cell: (4634, 4537),
        life: 0x80,
        path_target: (1, 2),
        ..ModeInput::default()
    };
    assert!(matches!(mode_message(&i), ModeMessage::Stop(_)));
    assert_eq!(send(&i), hex("6d 06000000 1a12 b911 80"));
    let b = ModeInput {
        mode: mode::BL,
        guid: 0x22,
        cell: (7, 8),
        path_target: (9, 10),
        direction: 3,
        unit_b0: 4,
        ..ModeInput::default()
    };
    assert_eq!(send(&b), hex("69 22000000 12 0000 ffff 00 00"));
    // With a target: 0x6A, code to unit (18), d = 0.
    let t = ModeInput {
        target: Some((0, 1)),
        ..b
    };
    assert_eq!(send(&t), hex("6a 22000000 12 00 01000000 00"));
}

/// The moving, attack and action vectors of the build's step 5 (measured
/// frames 24, 3149, 3080, 177).
#[test]
fn builder_vectors() {
    // (a, b) of a walk to a point is the path target (step 4, WL takes
    // it from the path): the measured bytes give the target, not the cell.
    let walk = ModeInput {
        mode: mode::WL,
        guid: 6,
        cell: (4820, 5630),
        path_target: (4825, 5636),
        stop_distance: 0,
        path_type: 7,
        velocity: 75,
        max_distance: 5,
        ..ModeInput::default()
    };
    assert_eq!(
        send(&walk),
        hex("67 06000000 01 d912 0416 01 00 07 4b00 05")
    );
    let chase = ModeInput {
        mode: mode::WL,
        guid: 0x23,
        cell: (4671, 5397),
        target: Some((0, 1)),
        path_type: 13,
        velocity: 75,
        max_distance: 5,
        ..ModeInput::default()
    };
    assert_eq!(
        send(&chase),
        hex("68 23000000 00 3f12 1515 00 01000000 01 00 0d 4b00 05")
    );
    let hit = ModeInput {
        mode: 4,
        guid: 0x26,
        cell: (4700, 5340),
        target: Some((1, 0x29)),
        ..ModeInput::default()
    };
    assert_eq!(send(&hit), hex("6c 26000000 0a 01 29000000 00 5c12 dc14"));
    let cast = ModeInput {
        mode: 8,
        guid: 6,
        cell: (4825, 5636),
        ..ModeInput::default()
    };
    assert_eq!(send(&cast), hex("6b 06000000 0c 0000 0000 00 00 d912 0416"));
}

/// Kashya's measured map walk at the Rogue Encampment arrival (seed
/// 1234, tick 32): from her
/// cell (4891, 4226) toward the map node (4898, 4233); the 0x67 carries
/// the node.
#[test]
fn a_walk_to_a_point_sends_the_path_target() {
    let i = ModeInput {
        mode: mode::WL,
        guid: 3,
        cell: (4891, 4226),
        path_target: (4898, 4233),
        stop_distance: 0,
        path_type: 7,
        velocity: 75,
        max_distance: 5,
        ..ModeInput::default()
    };
    assert_eq!(send(&i), hex("67 03000000 01 2213 8910 01 00 07 4b00 05"));
}

/// Step 3: mode 14 or a skill in use → the skill message (0x4C with the
/// target, 0x4D without); mode 14 with no skill → nothing.
#[test]
fn skill_in_use_is_not_a_mode_message() {
    let i = ModeInput {
        mode: 4,
        skill_in_use: true,
        target: Some((0, 1)),
        ..ModeInput::default()
    };
    assert_eq!(mode_message(&i), ModeMessage::Skill { to_unit: true });
    let j = ModeInput {
        mode: mode::SQ,
        skill_in_use: true,
        ..ModeInput::default()
    };
    assert_eq!(mode_message(&j), ModeMessage::Skill { to_unit: false });
    let k = ModeInput {
        mode: mode::SQ,
        ..ModeInput::default()
    };
    assert_eq!(mode_message(&k), ModeMessage::Nothing);
}

/// Step 4: modes 2 / 15 with a target on a path of type 5 or 6 send 0x67
/// to the path target (t' 1); type 8 becomes 11; 0x68's rewrite is 2 for
/// types 5 / 6. Step 5: s = stop distance + 1; the velocity is clamped.
#[test]
fn moving_forms_follow_the_path_type() {
    let base = ModeInput {
        mode: mode::RN,
        guid: 9,
        cell: (100, 200),
        path_target: (110, 210),
        target: Some((0, 1)),
        stop_distance: 2,
        max_distance: 7,
        velocity: 40_000,
        ..ModeInput::default()
    };
    let typed = ModeInput {
        path_type: 5,
        ..base
    };
    assert_eq!(
        send(&typed),
        hex("67 09000000 17 6e00 d200 03 00 01 ff7f 07")
    );
    let eight = ModeInput {
        path_type: 8,
        velocity: -40_000,
        ..base
    };
    assert_eq!(
        send(&eight),
        hex("68 09000000 18 6400 c800 00 01000000 03 00 0b 0080 07")
    );
    let plain = ModeInput {
        path_type: 6,
        target: None,
        ..base
    };
    // No target: 0x67 with the path target for type 6.
    assert_eq!(
        send(&plain),
        hex("67 09000000 17 6e00 d200 03 00 01 ff7f 07")
    );
}

/// Mode 13 (knockback): no target (its row has no "use target"), d =
/// path +0x90, f = unit +0xB0, e = the life fraction; mode 3: d = the
/// life fraction − 1 (above 1) | 0x80 with the unit's flag 0x100 set.
#[test]
fn knockback_and_get_hit() {
    let kb = ModeInput {
        mode: mode::KB,
        guid: 2,
        cell: (10, 11),
        path_target: (12, 13),
        path_90: 0x21,
        unit_b0: 5,
        life: 0x40,
        path_type: 4,
        velocity: 100,
        ..ModeInput::default()
    };
    assert_eq!(send(&kb), hex("67 02000000 14 0a00 0b00 21 05 04 6400 40"));
    let gh = ModeInput {
        mode: mode::GH,
        guid: 2,
        cell: (10, 11),
        path_target: (12, 13),
        life: 0x40,
        flag_100: true,
        unit_b0: 5,
        ..ModeInput::default()
    };
    assert_eq!(send(&gh), hex("69 02000000 06 0a00 0b00 bf 05"));
    let low = ModeInput {
        life: 1,
        flag_100: false,
        ..gh
    };
    assert_eq!(send(&low), hex("69 02000000 06 0a00 0b00 01 05"));
}

/// The builders write every byte the layout lists and zero the rest
/// (sizes 16, 21, 12, 12, 16, 16, 10).
#[test]
fn builder_sizes_and_ids() {
    let ids: Vec<(u8, usize)> = [
        monster_move(Move {
            guid: 0,
            code: 0,
            x: 0,
            y: 0,
            s: 0,
            t: 0,
            velocity: 0,
            max_distance: 0,
        })
        .to_vec(),
        move_to_unit(MoveToUnit {
            guid: 0,
            code: 0,
            x: 0,
            y: 0,
            a: 0,
            b: 0,
            s: 0,
            t: 0,
            velocity: 0,
            max_distance: 0,
        })
        .to_vec(),
        monster_state(0, 0, 0, 0, 0, 0).to_vec(),
        state_to_unit(0, 0, 0, 0, 0).to_vec(),
        monster_action(0, 0, 0, 0, 0, 0, 0, 0).to_vec(),
        monster_attack(0, 0, 0, 0, 0, 0, 0).to_vec(),
        monster_stop(0, 0, 0, 0).to_vec(),
    ]
    .iter()
    .map(|m| (m[0], m.len()))
    .collect();
    assert_eq!(
        ids,
        [
            (0x67, 16),
            (0x68, 21),
            (0x69, 12),
            (0x6A, 12),
            (0x6B, 16),
            (0x6C, 16),
            (0x6D, 10)
        ]
    );
}
