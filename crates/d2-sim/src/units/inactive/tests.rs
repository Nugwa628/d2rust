//! Synthetic checks of the compress decisions and the inactive records.

use super::*;

fn facts(ty: UnitType) -> CompressFacts {
    CompressFacts {
        ty,
        class: 1,
        mode: 1,
        save: false,
        player_body: false,
        object_no_restore: false,
        object_byte78_2: false,
        object_restore_virgins: false,
    }
}

#[test]
fn compress_table_per_type() {
    // S from SaveMonsters or unit flag 0x2000000.
    assert!(save_flag(true, 0) && save_flag(false, UNIT_FLAG_2000000));
    assert!(!save_flag(false, 0x100_0000));
    // Player body: cancel, kept, store, detach; not freed.
    let p = compress_other(&CompressFacts {
        player_body: true,
        ..facts(UnitType::Player)
    });
    assert!(p.cancel_events && p.mark_kept && p.store && p.detach && !p.free);
    // Other players: store if S, free.
    let p = compress_other(&facts(UnitType::Player));
    assert!(!p.store && p.free && !p.detach);
    let p = compress_other(&CompressFacts {
        save: true,
        ..facts(UnitType::Player)
    });
    assert!(p.store && p.free);
    // Portals 59 / 60 as a player body.
    for c in PORTAL_CLASSES {
        let o = compress_other(&CompressFacts {
            class: c,
            ..facts(UnitType::Object)
        });
        assert!(o.mark_kept && o.store && o.detach && !o.free);
    }
    // Objects: store := S, cleared by Restore 0, byte 0x78 & 2,
    // RestoreVirgins with mode ≠ 0; always cancel and free.
    let base = CompressFacts {
        save: true,
        ..facts(UnitType::Object)
    };
    let o = compress_other(&base);
    assert!(o.cancel_events && o.store && o.free && !o.mark_kept);
    for f in [
        CompressFacts {
            object_no_restore: true,
            ..base
        },
        CompressFacts {
            object_byte78_2: true,
            ..base
        },
        CompressFacts {
            object_restore_virgins: true,
            ..base
        },
    ] {
        assert!(!compress_other(&f).store);
    }
    let virgin_mode0 = CompressFacts {
        object_restore_virgins: true,
        mode: 0,
        ..base
    };
    assert!(compress_other(&virgin_mode0).store);
    // Missiles free, no store; items store, not freed here; tiles both.
    let m = compress_other(&facts(UnitType::Missile));
    assert!(m.free && !m.store);
    let i = compress_other(&facts(UnitType::Item));
    assert!(i.store && !i.free);
    let t = compress_other(&facts(UnitType::Tile));
    assert!(t.store && t.free);
}

fn mon() -> MonsterFacts {
    MonsterFacts {
        save: true,
        mode: 1,
        dead: false,
        udead_state: false,
        alignment: 0,
        node_index: 11,
        class: 5,
        type_flags: 0,
        unit_flags: 0,
        player_pet: false,
        restore: Some(1),
    }
}

use crate::rng::Seed;

fn seed() -> Seed {
    Seed::new(12345, 666)
}

#[test]
fn dead_monster_steps_the_room_seed() {
    let dead12 = MonsterFacts {
        mode: MODE_DEAD,
        ..mon()
    };
    // lo' 22752887: mod 3 = 2 → K := 0; one step.
    let mut s = seed();
    assert!(!compress_monster(&dead12, || Some(s.step())).store);
    let mut want = seed();
    want.step();
    assert_eq!(s, want);
    // A multiple of 3 keeps K; no seed clears it.
    assert!(compress_monster(&dead12, || Some(3)).store);
    assert!(!compress_monster(&dead12, || None).store);
    // Not mode 12: no step.
    let mut called = false;
    compress_monster(&mon(), || {
        called = true;
        None
    });
    assert!(!called);
}

#[test]
fn dead_monsters_with_udead_states_alignment_or_rank_are_not_stored() {
    let dead = MonsterFacts {
        dead: true,
        mode: 0,
        ..mon()
    };
    assert!(compress_monster(&dead, || None).store);
    for f in [
        MonsterFacts {
            udead_state: true,
            ..dead
        },
        MonsterFacts {
            alignment: 2,
            ..dead
        },
        MonsterFacts {
            class: 0x16B,
            ..dead
        },
        MonsterFacts {
            type_flags: 0x8,
            ..dead
        },
        MonsterFacts {
            type_flags: 0x10,
            ..dead
        },
    ] {
        assert!(!compress_monster(&f, || None).store, "{f:?}");
    }
    // Step 8 restore 2 overrides them all.
    let f = MonsterFacts {
        udead_state: true,
        restore: Some(2),
        ..dead
    };
    assert!(compress_monster(&f, || None).store);
}

#[test]
fn node_index_below_8() {
    let alive = MonsterFacts {
        save: false,
        node_index: 3,
        ..mon()
    };
    let c = compress_monster(&alive, || None);
    assert!(c.store && c.neutral_mode && c.free);
    // Step 4 is skipped for it (class 0x16B alive is not dead anyway).
    let dead = MonsterFacts {
        dead: true,
        node_index: 3,
        ..mon()
    };
    let c = compress_monster(&dead, || None);
    assert!(!c.store && !c.neutral_mode);
}

#[test]
fn flags_restore_column_and_pets() {
    for f in [
        MonsterFacts {
            unit_flags: UNIT_FLAG_200,
            ..mon()
        },
        MonsterFacts {
            restore: Some(0),
            ..mon()
        },
        MonsterFacts {
            restore: None,
            ..mon()
        },
    ] {
        let c = compress_monster(&f, || None);
        assert!(!c.store && c.free, "{f:?}");
    }
    // restore 2 sets K even without S; 1 keeps it.
    let c = compress_monster(
        &MonsterFacts {
            save: false,
            restore: Some(2),
            ..mon()
        },
        || None,
    );
    assert!(c.store);
    // Bit 31: K := 0; a player's pet is kept (detached, not freed).
    let pet = MonsterFacts {
        unit_flags: UNIT_FLAG_BIT31,
        player_pet: true,
        ..mon()
    };
    let c = compress_monster(&pet, || None);
    assert!(!c.store && c.mark_kept && c.detach && !c.free);
    let c = compress_monster(
        &MonsterFacts {
            restore: Some(2),
            ..pet
        },
        || None,
    );
    assert!(c.store && c.detach);
    // P only with bit 31.
    let c = compress_monster(
        &MonsterFacts {
            player_pet: true,
            ..mon()
        },
        || None,
    );
    assert!(c.free && !c.detach);
}

#[test]
fn nodes_descend_in_x_and_records_push_at_the_head() {
    let mut s = InactiveStore::default();
    s.push_other(1, (10, 0), OtherRecord::default());
    s.push_other(1, (30, 0), OtherRecord::default());
    s.push_other(1, (20, 5), OtherRecord::default());
    s.push_other(1, (20, 0), OtherRecord::default());
    let xs: Vec<(i32, i32)> = s.acts[1].iter().map(|n| (n.x, n.y)).collect();
    // A new node goes before the first node of smaller x.
    assert_eq!(xs, [(30, 0), (20, 5), (20, 0), (10, 0)]);
    s.push_monster(
        1,
        (10, 0),
        MonsterRecord {
            guid: 1,
            ..MonsterRecord::default()
        },
    );
    s.push_monster(
        1,
        (10, 0),
        MonsterRecord {
            guid: 2,
            ..MonsterRecord::default()
        },
    );
    let n = s.take(1, 10, 0).unwrap();
    assert_eq!(
        n.monsters.iter().map(|m| m.guid).collect::<Vec<_>>(),
        [2, 1]
    );
    assert!(s.take(1, 10, 0).is_none());
    assert!(s.acts[0].is_empty());
}

#[test]
fn monster_record_bits() {
    let b = monster_bits(true, true, MODE_DEAD, 0x18 | 0x1, true, true, 1, 3, true);
    assert_eq!(
        b,
        mrec::TYPE_FLAG_1
            | mrec::CHAMPION
            | mrec::DEAD
            | mrec::OWNER_8
            | mrec::OWNER_10
            | mrec::MINION
            | mrec::UNIT_573540
            | mrec::ALIGN_1
            | mrec::NODE_NOT_11
            | mrec::SUPERUNIQUE
    );
    assert_eq!(
        monster_bits(false, false, 0, 0, false, false, 2, 11, false),
        mrec::DEAD | mrec::ALIGN_2
    );
    assert_eq!(
        monster_bits(false, false, 1, 0, false, false, 0, 11, false),
        mrec::ALIGN_0
    );
}

#[test]
fn item_records_expire_or_get_a_fresh_ground_time() {
    // Expired: dropped.
    assert_eq!(restore_item_expiry(99, 100), None);
    // Never: stays 0. Far enough: kept. Near: frame + 15,000.
    assert_eq!(restore_item_expiry(0, 100), Some(0));
    assert_eq!(restore_item_expiry(20_000, 100), Some(20_000));
    assert_eq!(restore_item_expiry(15_100, 100), Some(15_100));
    assert_eq!(restore_item_expiry(15_099, 100), Some(15_100));
    assert_eq!(restore_item_expiry(100, 100), Some(15_100));
}

fn rec(bits: u32) -> MonsterRecord {
    MonsterRecord {
        class: 7,
        bits,
        ..MonsterRecord::default()
    }
}

#[test]
fn restore_order_and_monster_routes() {
    let node = AreaNode {
        x: 0,
        y: 0,
        items: vec![ItemRecord::default()],
        monsters: vec![rec(0), rec(mrec::DEAD)],
        others: vec![OtherRecord::default()],
    };
    let o = restore_order(node.clone());
    assert_eq!(o.monsters, node.monsters);
    assert_eq!((o.items.len(), o.others.len()), (1, 1));
    assert_eq!(
        restore_monster(&rec(0), false),
        MonsterRestore::Spawn {
            kind: SpawnKind::Plain,
            mode: MODE_NEUTRAL
        }
    );
    assert_eq!(
        restore_monster(&rec(mrec::DEAD | mrec::MINION | mrec::TYPE_FLAG_1), false),
        MonsterRestore::Spawn {
            kind: SpawnKind::Minion,
            mode: MODE_DEAD
        }
    );
    assert_eq!(
        restore_monster(&rec(mrec::TYPE_FLAG_1), false),
        MonsterRestore::Spawn {
            kind: SpawnKind::Unique,
            mode: MODE_NEUTRAL
        }
    );
    // Kept pets are re-placed.
    let kept = MonsterRecord {
        flags_ex: FLAGS2_KEPT,
        ..rec(0)
    };
    assert_eq!(restore_monster(&kept, false), MonsterRestore::Replace);
    // 0x400 with alignment 1 or 2: skipped; alone or with alignment 0: not.
    for a in [mrec::ALIGN_1, mrec::ALIGN_2] {
        assert_eq!(
            restore_monster(&rec(mrec::NODE_NOT_11 | a), false),
            MonsterRestore::Skip
        );
    }
    assert_ne!(
        restore_monster(&rec(mrec::NODE_NOT_11 | mrec::ALIGN_0), false),
        MonsterRestore::Skip
    );
    // Level 108 gate: only class 243.
    assert_eq!(restore_monster(&rec(0), true), MonsterRestore::Skip);
    let diablo = MonsterRecord {
        class: SANCTUARY_CLASS,
        ..rec(0)
    };
    assert_ne!(restore_monster(&diablo, true), MonsterRestore::Skip);
}
