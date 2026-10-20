//! Unit tests of the monster palette shift, the shift map choice and the blood map (`rules::shading`), on synthetic map
//! data (repo only; the game's `palshift.dat`, `RandTransforms.dat` and
//! `GreenBlood.dat` are never embedded).

use super::shading::{
    blood_map, green_blood_switch, monster_shift_index, palshift_map, rand_transforms_map,
    shift_map, shift_trans_lvl, shift_unique, shift_utrans, ShiftError, ShiftInputs, ShiftMap,
    ShiftMapInputs,
};

/// `n` maps, map `k` filled with byte `k + 1` (after `lead` lead bytes).
fn maps(lead: usize, n: usize) -> Vec<u8> {
    let mut v = vec![0xEE; lead];
    for k in 0..n {
        v.extend(std::iter::repeat_n((k + 1) as u8, 256));
    }
    v
}

fn monster(class: u32) -> ShiftMapInputs {
    ShiftMapInputs {
        unit_type: 1,
        class,
        has_palshift: true,
        green_blood: false,
        local_blood: 0,
        rand_transforms_loaded: true,
    }
}

#[test]
fn unique_vector_class_5_name_seed_0x1234() {
    let m = ShiftInputs {
        class: 5,
        name_seed: 0x1234,
        unique: true,
        ..ShiftInputs::default()
    };
    assert_eq!(shift_unique(5, 0x1234), 20);
    let s = monster_shift_index(&m);
    assert_eq!(s, 20);
    assert_eq!(shift_map(s, &monster(5)), Ok(ShiftMap::RandTransforms(12)));
}

#[test]
fn trans_lvl_step() {
    assert_eq!(shift_trans_lvl(0), 2);
    assert_eq!(shift_trans_lvl(5), 7);
    assert_eq!(shift_trans_lvl(7), 9);
    assert_eq!(shift_trans_lvl(8), 2);
    assert_eq!(shift_trans_lvl(200), 2);
    let m = ShiftInputs {
        trans_lvl: 8,
        ..ShiftInputs::default()
    };
    assert_eq!(monster_shift_index(&m), 2);
    assert_eq!(
        shift_map(2, &monster(5)),
        Ok(ShiftMap::Palshift { offset: 4 + 512 })
    );
}

#[test]
fn unique_step_needs_unique_and_shift_allowed() {
    let base = ShiftInputs {
        class: 5,
        name_seed: 0x1234,
        trans_lvl: 3,
        ..ShiftInputs::default()
    };
    assert_eq!(monster_shift_index(&base), 5);
    let blocked = ShiftInputs {
        unique: true,
        no_unique_shift: true,
        ..base
    };
    assert_eq!(monster_shift_index(&blocked), 5);
    // Class + name seed wraps as u32; the result is always 9…38.
    for (c, n) in [(0, 0), (u32::MAX, 0xFFFF), (700, 1)] {
        assert!((9..39).contains(&shift_unique(c, n)));
    }
}

#[test]
fn utrans_and_superunique_override_then_clamp() {
    assert_eq!(shift_utrans(0, false), None);
    assert_eq!(shift_utrans(5, true), Some(5));
    assert_eq!(shift_utrans(255, false), Some(1));
    assert_eq!(shift_utrans(255, true), Some(0));
    let u = ShiftInputs {
        class: 5,
        name_seed: 0x1234,
        unique: true,
        utrans: 5,
        ..ShiftInputs::default()
    };
    assert_eq!(monster_shift_index(&u), 5);
    let su = ShiftInputs {
        superunique_utrans: Some(12),
        ..u
    };
    assert_eq!(monster_shift_index(&su), 12);
    let su0 = ShiftInputs {
        superunique_utrans: Some(0),
        ..u
    };
    assert_eq!(monster_shift_index(&su0), 5);
    // Step 5: s ≥ 30 → 2 (the unique step reaches 38).
    let big = ShiftInputs {
        utrans: 31,
        ..ShiftInputs::default()
    };
    assert_eq!(monster_shift_index(&big), 2);
    let edge = ShiftInputs {
        utrans: 29,
        ..ShiftInputs::default()
    };
    assert_eq!(monster_shift_index(&edge), 29);
}

#[test]
fn map_choice_rules() {
    // Not a monster, or no palshift data: none.
    let other = ShiftMapInputs {
        unit_type: 0,
        ..monster(5)
    };
    assert_eq!(shift_map(4, &other), Ok(ShiftMap::None));
    let no_data = ShiftMapInputs {
        has_palshift: false,
        ..monster(5)
    };
    assert_eq!(shift_map(4, &no_data), Ok(ShiftMap::None));
    // s 0, 1: only classes 363 and 364.
    assert_eq!(shift_map(0, &monster(5)), Ok(ShiftMap::None));
    assert_eq!(shift_map(1, &monster(362)), Ok(ShiftMap::None));
    assert_eq!(
        shift_map(1, &monster(363)),
        Ok(ShiftMap::Palshift { offset: 4 + 256 })
    );
    assert_eq!(
        shift_map(0, &monster(364)),
        Ok(ShiftMap::Palshift { offset: 4 })
    );
    // s 2…7: first set; second set with green blood and localBlood 2.
    assert_eq!(
        shift_map(7, &monster(5)),
        Ok(ShiftMap::Palshift {
            offset: 4 + 7 * 256
        })
    );
    let green = ShiftMapInputs {
        green_blood: true,
        local_blood: 2,
        ..monster(5)
    };
    assert_eq!(
        shift_map(3, &green),
        Ok(ShiftMap::Palshift {
            offset: 0x804 + 3 * 256
        })
    );
    let green1 = ShiftMapInputs {
        local_blood: 1,
        ..green
    };
    assert_eq!(
        shift_map(3, &green1),
        Ok(ShiftMap::Palshift {
            offset: 4 + 3 * 256
        })
    );
    // s ≥ 8: RandTransforms map s − 8; past 29 fatal 0x160.
    assert_eq!(shift_map(8, &monster(5)), Ok(ShiftMap::RandTransforms(0)));
    assert_eq!(shift_map(37, &monster(5)), Ok(ShiftMap::RandTransforms(29)));
    assert_eq!(
        shift_map(38, &monster(5)),
        Err(ShiftError::RandTransformsFatal(30))
    );
    // s ≥ 8 without RandTransforms: the palshift rule with that s.
    let no_rt = ShiftMapInputs {
        rand_transforms_loaded: false,
        ..monster(5)
    };
    assert_eq!(
        shift_map(9, &no_rt),
        Ok(ShiftMap::Palshift {
            offset: 4 + 9 * 256
        })
    );
}

#[test]
fn map_bytes_from_caller_data() {
    // A class block: 4 lead bytes, 8 maps, then the second set at 0x804.
    let block = maps(4, 16);
    let Ok(ShiftMap::Palshift { offset }) = shift_map(2, &monster(5)) else {
        panic!("palshift map");
    };
    assert_eq!(palshift_map(&block, offset).unwrap()[0], 3);
    assert_eq!(palshift_map(&block, 0x804 + 256).unwrap()[255], 10);
    assert_eq!(
        palshift_map(&block, 4 + 16 * 256),
        Err(ShiftError::PastData {
            what: "palshift",
            offset: 4 + 16 * 256,
            len: block.len()
        })
    );
    let rt = maps(0, 30);
    assert_eq!(rand_transforms_map(&rt, 12).unwrap()[100], 13);
    assert!(rand_transforms_map(&rt[..256 * 12], 12).is_err());
    assert_eq!(
        rand_transforms_map(&rt, 30),
        Err(ShiftError::RandTransformsFatal(30))
    );
}

#[test]
fn blood_map_and_green_blood_switch() {
    assert!(green_blood_switch(Some(b"1")));
    assert!(green_blood_switch(Some(b"1abc")));
    assert!(!green_blood_switch(Some(b"0")));
    assert!(!green_blood_switch(Some(b" 1")));
    assert!(!green_blood_switch(Some(b"")));
    assert!(!green_blood_switch(None));
    let gb = maps(0, 1);
    assert_eq!(blood_map(&gb, false, true), Ok(None));
    assert_eq!(blood_map(&gb, true, false), Ok(None));
    assert_eq!(blood_map(&gb, true, true).unwrap().unwrap()[0], 1);
    assert!(blood_map(&gb[..255], true, true).is_err());
}
