//! Per-entry table checks, one test per entry section.

use super::tables::{entry, table_of};

#[test]
fn entry_1_den_of_evil() {
    let t = table_of(1).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (1, 1, 0, 0));
    assert_eq!((t.title, t.completed, t.pending), (3714, 76, 0x4));
    assert_eq!(entry(1).filter, 1);
    let expect: [(u16, u16); 14] = [
        (3735, 64),
        (3736, 64),
        (3737, 64),
        (3738, 64),
        (3740, 64),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3740, 64),
        (3728, 3725),
        (3727, 3725),
        (3726, 64),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_2_sisters_burial_grounds() {
    let t = table_of(2).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (2, 1, 1, 1));
    assert_eq!((t.title, t.completed, t.pending), (3715, 92, 0x2));
    assert_eq!(entry(2).filter, 2);
    let expect: [(u16, u16); 14] = [
        (3741, 81),
        (3742, 81),
        (3743, 81),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3743, 81),
        (3728, 3725),
        (3727, 3725),
        (3726, 81),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_3_tools_of_the_trade() {
    let t = table_of(3).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (3, 1, 4, 2));
    assert_eq!((t.title, t.completed, t.pending), (3716, 163, 0xFFFF));
    assert_eq!(entry(3).filter, 3);
    let expect: [(u16, u16); 14] = [
        (3755, 146),
        (3756, 146),
        (3733, 146),
        (3732, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3757, 163),
        (3728, 3725),
        (3727, 3725),
        (3726, 146),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_4_the_search_for_cain() {
    let t = table_of(4).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (4, 1, 2, 3));
    assert_eq!((t.title, t.completed, t.pending), (3717, 123, 0xFFFF));
    assert_eq!(entry(4).filter, 4);
    let expect: [(u16, u16); 14] = [
        (3744, 97),
        (3745, 97),
        (3746, 97),
        (3747, 97),
        (3748, 97),
        (3749, 97),
        (3734, 97),
        (3725, 3725),
        (3725, 3725),
        (3750, 97),
        (3728, 3725),
        (3727, 3725),
        (3726, 97),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_5_the_forgotten_tower() {
    let t = table_of(5).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (5, 1, 3, 4));
    assert_eq!((t.title, t.completed, t.pending), (3718, 127, 0xFFFF));
    assert_eq!(entry(5).filter, 5);
    let expect: [(u16, u16); 14] = [
        (3751, 127),
        (3754, 127),
        (3752, 127),
        (3753, 127),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 127),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_6_sisters_to_the_slaughter() {
    let t = table_of(6).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (6, 1, 5, 5));
    assert_eq!((t.title, t.completed, t.pending), (3719, 184, 0x9));
    assert_eq!(entry(6).filter, 6);
    let expect: [(u16, u16); 14] = [
        (3758, 166),
        (3759, 166),
        (3761, 166),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3760, 166),
        (3728, 3725),
        (3727, 3725),
        (3726, 184),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_9_radament_s_lair() {
    let t = table_of(9).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (8, 2, 0, 6));
    assert_eq!((t.title, t.completed, t.pending), (923, 334, 0x2));
    assert_eq!(entry(9).filter, 9);
    let expect: [(u16, u16); 14] = [
        (941, 304),
        (942, 304),
        (943, 304),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (943, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 304),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_10_the_horadric_staff() {
    let t = table_of(10).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (9, 2, 1, 7));
    assert_eq!((t.title, t.completed, t.pending), (924, 339, 0xFFFF));
    assert_eq!(entry(10).filter, 10);
    let expect: [(u16, u16); 14] = [
        (944, 3725),
        (945, 335),
        (946, 338),
        (948, 3725),
        (947, 339),
        (947, 3725),
        (3725, 3725),
        (3725, 3725),
        (3730, 3725),
        (947, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 339),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_11_tainted_sun() {
    let t = table_of(11).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (10, 2, 2, 8));
    assert_eq!((t.title, t.completed, t.pending), (925, 3725, 0xFFFF));
    assert_eq!(entry(11).filter, 11);
    let expect: [(u16, u16); 14] = [
        (950, 3725),
        (951, 3725),
        (952, 348),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (952, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_12_arcane_sanctuary() {
    let t = table_of(12).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (11, 2, 3, 9));
    assert_eq!((t.title, t.completed, t.pending), (926, 3725, 0xFFFF));
    assert_eq!(entry(12).filter, 12);
    let expect: [(u16, u16); 14] = [
        (954, 3725),
        (953, 373),
        (953, 377),
        (955, 377),
        (956, 396),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (956, 396),
        (3728, 396),
        (3727, 3725),
        (3726, 396),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_13_the_summoner() {
    let t = table_of(13).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (12, 2, 4, 10));
    assert_eq!((t.title, t.completed, t.pending), (927, 3725, 0xFFFF));
    assert_eq!(entry(13).filter, 13);
    let expect: [(u16, u16); 14] = [
        (957, 3725),
        (958, 3725),
        (3726, 3725),
        (959, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (959, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_14_the_seven_tombs() {
    let t = table_of(14).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (13, 2, 5, 11));
    assert_eq!((t.title, t.completed, t.pending), (928, 442, 0x4));
    assert_eq!(entry(14).filter, 14);
    let expect: [(u16, u16); 14] = [
        (960, 430),
        (962, 431),
        (11030, 431),
        (964, 431),
        (965, 302),
        (966, 442),
        (961, 396),
        (3729, 3725),
        (3730, 3725),
        (965, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 442),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_17_lam_esen_s_tome() {
    let t = table_of(17).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (15, 3, 3, 12));
    assert_eq!((t.title, t.completed, t.pending), (930, 564, 0xFFFF));
    assert_eq!(entry(17).filter, 17);
    let expect: [(u16, u16); 14] = [
        (968, 549),
        (969, 549),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3730, 3725),
        (3729, 3725),
        (3725, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_18_khalim_s_will() {
    let t = table_of(18).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (16, 3, 2, 13));
    assert_eq!((t.title, t.completed, t.pending), (931, 548, 0xFFFF));
    assert_eq!(entry(18).filter, 18);
    let expect: [(u16, u16); 14] = [
        (970, 543),
        (971, 543),
        (972, 543),
        (973, 543),
        (974, 543),
        (975, 548),
        (976, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_19_blade_of_the_old_religion() {
    let t = table_of(19).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (17, 3, 1, 14));
    assert_eq!((t.title, t.completed, t.pending), (932, 587, 0xFFFF));
    assert_eq!(entry(19).filter, 19);
    let expect: [(u16, u16); 14] = [
        (993, 3725),
        (977, 571),
        (978, 571),
        (979, 571),
        (980, 587),
        (981, 587),
        (3730, 3725),
        (3729, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 587),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_20_the_golden_bird() {
    let t = table_of(20).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (18, 3, 0, 15));
    assert_eq!((t.title, t.completed, t.pending), (933, 3725, 0x4));
    assert_eq!(entry(20).filter, 20);
    let expect: [(u16, u16); 14] = [
        (982, 3725),
        (983, 527),
        (984, 529),
        (985, 531),
        (986, 534),
        (3729, 3725),
        (3730, 3725),
        (3725, 3725),
        (3725, 3725),
        (986, 534),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_21_the_blackened_temple() {
    let t = table_of(21).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (19, 3, 4, 16));
    assert_eq!((t.title, t.completed, t.pending), (934, 3725, 0x3));
    assert_eq!(entry(21).filter, 21);
    let expect: [(u16, u16); 14] = [
        (994, 3725),
        (987, 594),
        (988, 594),
        (989, 594),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (989, 594),
        (3728, 626),
        (3727, 3725),
        (3726, 626),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_22_the_guardian() {
    let t = table_of(22).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (20, 3, 5, 17));
    assert_eq!((t.title, t.completed, t.pending), (935, 3725, 0x4));
    assert_eq!(entry(22).filter, 22);
    let expect: [(u16, u16); 14] = [
        (995, 3725),
        (990, 628),
        (991, 628),
        (992, 628),
        (935, 628),
        (3729, 3725),
        (3730, 3725),
        (3725, 3725),
        (3725, 3725),
        (3728, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_25_the_fallen_angel() {
    let t = table_of(25).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (22, 4, 0, 18));
    assert_eq!((t.title, t.completed, t.pending), (937, 675, 0xFFFF));
    assert_eq!(entry(25).filter, 25);
    let expect: [(u16, u16); 14] = [
        (996, 670),
        (997, 670),
        (998, 670),
        (999, 675),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (999, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 676),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_26_terror_s_end() {
    let t = table_of(26).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (23, 4, 2, 20));
    assert_eq!((t.title, t.completed, t.pending), (938, 3725, 0xFFFF));
    assert_eq!(entry(26).filter, 26);
    let expect: [(u16, u16); 14] = [
        (1004, 681),
        (1005, 681),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_27_hell_s_forge() {
    let t = table_of(27).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (24, 4, 1, 19));
    assert_eq!((t.title, t.completed, t.pending), (939, 3725, 0xFFFF));
    assert_eq!(entry(27).filter, 27);
    let expect: [(u16, u16); 14] = [
        (1000, 678),
        (1001, 678),
        (1002, 678),
        (1000, 679),
        (3725, 3725),
        (3729, 3725),
        (3730, 3725),
        (3725, 3725),
        (3725, 3725),
        (3728, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_35_siege_on_harrogath() {
    let t = table_of(35).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (31, 5, 0, 21));
    assert_eq!((t.title, t.completed, t.pending), (22618, 3725, 0xFFFF));
    assert_eq!(entry(35).filter, 35);
    let expect: [(u16, u16); 14] = [
        (22619, 20077),
        (22620, 20077),
        (22621, 20077),
        (21786, 20090),
        (3725, 3725),
        (3729, 3725),
        (3730, 3725),
        (3725, 3725),
        (3725, 3725),
        (21786, 20090),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_36_rescue_on_mount_arreat() {
    let t = table_of(36).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (32, 5, 1, 22));
    assert_eq!((t.title, t.completed, t.pending), (22622, 3725, 0xFFFF));
    assert_eq!(entry(36).filter, 36);
    let expect: [(u16, u16); 14] = [
        (22623, 20096),
        (22624, 20104),
        (22625, 20096),
        (22626, 20096),
        (21789, 20096),
        (3729, 3725),
        (3730, 3725),
        (3725, 3725),
        (3725, 3725),
        (22625, 20096),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_37_prison_of_ice() {
    let t = table_of(37).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (33, 5, 2, 23));
    assert_eq!((t.title, t.completed, t.pending), (22627, 3725, 0xFFFF));
    assert_eq!(entry(37).filter, 37);
    let expect: [(u16, u16); 14] = [
        (22628, 20116),
        (21788, 20116),
        (22629, 20116),
        (22630, 20116),
        (22631, 20116),
        (22632, 20116),
        (3730, 3725),
        (3725, 3725),
        (3725, 3725),
        (22631, 20116),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_38_betrayal_of_harrogath() {
    let t = table_of(38).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (34, 5, 3, 24));
    assert_eq!((t.title, t.completed, t.pending), (22633, 3725, 0xFFFF));
    assert_eq!(entry(38).filter, 38);
    let expect: [(u16, u16); 14] = [
        (22634, 20137),
        (21787, 20137),
        (22635, 20137),
        (22636, 20137),
        (21790, 20148),
        (3729, 3725),
        (3730, 3725),
        (3725, 3725),
        (3725, 3725),
        (21790, 3725),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_39_rite_of_passage() {
    let t = table_of(39).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (35, 5, 4, 25));
    assert_eq!((t.title, t.completed, t.pending), (22637, 3725, 0x3));
    assert_eq!(entry(39).filter, 39);
    let expect: [(u16, u16); 14] = [
        (22638, 20153),
        (22639, 20153),
        (22640, 20002),
        (22639, 20002),
        (3725, 3725),
        (3729, 3725),
        (3730, 3725),
        (3725, 3725),
        (3725, 3725),
        (22639, 20002),
        (3728, 3725),
        (3727, 3725),
        (3726, 3725),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}

#[test]
fn entry_40_eve_of_destruction() {
    let t = table_of(40).unwrap();
    assert_eq!((t.chain, t.tab + 1, t.slot, t.icon), (36, 5, 5, 26));
    assert_eq!((t.title, t.completed, t.pending), (22641, 3725, 0xFFFF));
    assert_eq!(entry(40).filter, 40);
    let expect: [(u16, u16); 14] = [
        (21792, 20169),
        (21881, 20169),
        (22643, 20169),
        (22644, 20169),
        (22645, 20169),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3725, 3725),
        (3728, 3725),
        (3728, 3725),
        (11, 13),
        (12, 14),
        (3725, 3725),
    ];
    for (i, want) in expect.iter().enumerate() {
        assert_eq!(t.row(i as u8 + 1), *want, "status {}", i + 1);
    }
}
