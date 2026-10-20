// Test vectors: (the parts that need no world);
// the bodies on the wired host: `crate::wiring::interaction` tests
// (`skill_bodies`).
use super::*;

#[test]
fn event_function_table_bounds() {
    assert!(!event_func_filled(0));
    for f in 1..=31 {
        assert!(event_func_filled(f));
    }
    assert!(!event_func_filled(32));
    assert!(!event_func_filled(50));
    assert!(!event_func_filled(-1));
}

#[test]
fn callback_ids_are_the_1_14d_addresses() {
    assert_eq!(callback::DEFAULT, 0x0056E900);
    assert_eq!(callback::SELF_AURA, 0x005CEC50);
    assert_eq!(callback::BUFF, 0x005C9420);
    assert_eq!(callback::AI_CURSE, 0x005C3370);
}

#[test]
fn constants() {
    assert_eq!(DEFAULT_FILTER, 0x583);
    assert_eq!(
        (group::PGSV, group::CURSE, group::CURABLE, group::EXP),
        (4, 11, 12, 30)
    );
    assert_eq!(
        (group::DISGUISE, group::UDEAD, group::MELEEONLY),
        (16, 33, 38)
    );
}

#[test]
fn aura_filter_73731_bits() {
    // 73731 = 0x12003: players, monsters, allies, no town rooms; no
    // 0x80 / 0x400 tests.
    let f = 73731u32;
    assert_eq!(f, 0x12003);
    assert_ne!(f & 0x1, 0);
    assert_ne!(f & 0x2, 0);
    assert_ne!(f & 0x10000, 0);
    assert_ne!(f & 0x2000, 0);
    assert_eq!(f & (0x80 | 0x400 | 0x8000), 0);
}

/// The body tables of the extracted the base game tables: one itemstatcost
/// record per stat, 185 states, life / mana /
/// stamina `direct` with their maxima as `maxstat`.
#[test]
#[ignore = "needs the game tables in D2_GAME_DIR"]
fn body_tables_from_the_1_14d_tables() {
    use crate::skills::tests_game as game;
    use d2_data::tables::{Itemstatcost, Overlay, Record, States};
    let isc = game::table("itemstatcost", Itemstatcost::SIZE);
    let st = game::table("states", States::SIZE);
    let ov = game::table("overlay", Overlay::SIZE);
    let b = BodyTables::from_tables(&isc, &st, &ov).expect("tables decode");
    assert_eq!(b.stats.len(), isc.count);
    assert_eq!(b.state_group.len(), 185);
    assert_eq!(b.overlay_count as usize, ov.count);
    for (s, max) in [(6, 7), (8, 9), (10, 11)] {
        let x = b.stat(s).unwrap();
        assert!(x.direct, "stat {s}");
        assert_eq!(x.maxstat, max, "stat {s}");
    }
    println!(
        "itemstatcost {}, overlays {}, state groups {:?}",
        isc.count,
        ov.count,
        b.state_group
            .iter()
            .enumerate()
            .filter(|(_, &g)| g != 0)
            .collect::<Vec<_>>()
    );
}
