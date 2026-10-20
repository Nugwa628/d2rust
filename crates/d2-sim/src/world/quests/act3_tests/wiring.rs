// Spec: specs/world/quests-act3.md §1.3, §2, §10 (shared Act III code)
//! The sequence chain, the init stores, the status iterate rule, the
//! creation links and the quest dispatch reaching Act III.

use super::*;

fn state(ctl: &QuestControl, chain: u8) -> u8 {
    ctl.record(chain).unwrap().state
}

fn set_state(ctl: &mut QuestControl, chain: u8, s: u8) {
    ctl.record_mut(chain).unwrap().state = s;
}

fn seq(ctl: &mut QuestControl, f: &mut Fake3, chain: u8) -> bool {
    act3::sequence(ctl, f, chain)
}

// ------------------------------------------------------------ §1.3

// Covers: specs/world/quests-act3.md §1.3
#[test]
fn sequence_golden_bird_root() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    // State ≠ 5 and not-intro: 1, nothing moves.
    for s in [0, 1, 4, 6] {
        set_state(&mut ctl, 18, s);
        assert!(seq(&mut ctl, &mut f, 18));
        assert_eq!((state(&ctl, 16), state(&ctl, 17)), (0, 0));
    }
    // State 5: seq(16) and seq(17), both 0 → 1.
    set_state(&mut ctl, 18, 5);
    assert!(seq(&mut ctl, &mut f, 18));
    assert_eq!((state(&ctl, 16), state(&ctl, 17)), (1, 1));
    // Not-intro 0 (any state): the same walk.
    let (mut ctl, _) = control();
    ctl.record_mut(18).unwrap().not_intro = false;
    assert!(seq(&mut ctl, &mut f, 18));
    assert_eq!((state(&ctl, 16), state(&ctl, 17)), (1, 1));
    // r1 or r2: one absent chain still returns the other's 1.
    let (mut ctl, _) = control();
    set_state(&mut ctl, 18, 5);
    ctl.records.retain(|r| r.chain != 16);
    assert!(seq(&mut ctl, &mut f, 18));
    assert_eq!(state(&ctl, 17), 1);
    // Both absent: 0.
    ctl.records.retain(|r| r.chain != 17);
    assert!(!seq(&mut ctl, &mut f, 18));
    assert!(f.log().is_empty());
}

// Covers: specs/world/quests-act3.md §1.3
#[test]
fn sequence_khalim_and_blade() {
    for chain in [16, 17] {
        let (mut ctl, _) = control();
        let mut f = Fake3::new();
        // State 0 → 1, returns 1.
        assert!(seq(&mut ctl, &mut f, chain));
        assert_eq!(state(&ctl, chain), 1);
        // Other states ≠ 5: 1, no change.
        set_state(&mut ctl, chain, 3);
        assert!(seq(&mut ctl, &mut f, chain));
        assert_eq!(state(&ctl, chain), 3);
        // State 5: seq(15) (state 0, not-intro → 1, no change).
        set_state(&mut ctl, chain, 5);
        assert!(seq(&mut ctl, &mut f, chain));
        assert_eq!((state(&ctl, chain), state(&ctl, 15)), (5, 0));
        // Not-intro 0 at state 0: no state change, seq(15).
        let (mut ctl, _) = control();
        ctl.record_mut(chain).unwrap().not_intro = false;
        ctl.record_mut(15).unwrap().not_intro = false;
        ctl.record_mut(19).unwrap().not_intro = false;
        ctl.record_mut(20).unwrap().not_intro = false;
        assert!(seq(&mut ctl, &mut f, chain));
        assert_eq!(state(&ctl, chain), 0);
        assert_eq!(state(&ctl, 20), 0);
        // seq(15) of an absent chain: 0.
        ctl.records.retain(|r| r.chain != 15);
        assert!(!seq(&mut ctl, &mut f, chain));
        assert!(f.log().is_empty());
    }
}

// Covers: specs/world/quests-act3.md §1.3
#[test]
fn sequence_lam_esen_and_temple() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    // 15: state ≠ 5 and not-intro → 1 with no state change.
    assert!(seq(&mut ctl, &mut f, 15));
    assert_eq!(state(&ctl, 15), 0);
    // 15 state 5 → seq(19): state ≠ 7 → 1; 20 untouched.
    set_state(&mut ctl, 15, 5);
    assert!(seq(&mut ctl, &mut f, 15));
    assert_eq!((state(&ctl, 19), state(&ctl, 20)), (0, 0));
    // 19 state 5 (its quest's 5 is not its pass state): still 1.
    set_state(&mut ctl, 19, 5);
    assert!(seq(&mut ctl, &mut f, 15));
    assert_eq!(state(&ctl, 20), 0);
    // 19 state 7 → seq(20): state 0 → 1 and callback 2.
    set_state(&mut ctl, 19, 7);
    assert!(!ctl.record(20).unwrap().has_callback(event::NPC_DEACTIVATE));
    assert!(seq(&mut ctl, &mut f, 15));
    assert_eq!(state(&ctl, 20), 1);
    assert!(ctl.record(20).unwrap().has_callback(event::NPC_DEACTIVATE));
    // 19 absent: 0.
    ctl.records.retain(|r| r.chain != 19);
    assert!(!seq(&mut ctl, &mut f, 15));
    assert!(f.log().is_empty());
}

// Covers: specs/world/quests-act3.md §1.3
#[test]
fn sequence_guardian() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    // State 0, not-intro: state 1, callback 2, returns 1.
    assert!(seq(&mut ctl, &mut f, 20));
    assert_eq!(state(&ctl, 20), 1);
    assert!(ctl.record(20).unwrap().has_callback(event::NPC_DEACTIVATE));
    // Nothing changed: still 1.
    ctl.record_mut(20)
        .unwrap()
        .clear_callback(event::NPC_DEACTIVATE);
    assert!(seq(&mut ctl, &mut f, 20));
    assert_eq!(state(&ctl, 20), 1);
    assert!(!ctl.record(20).unwrap().has_callback(event::NPC_DEACTIVATE));
    // Not-intro 0 at state 0: nothing, 1.
    let (mut ctl, _) = control();
    ctl.record_mut(20).unwrap().not_intro = false;
    assert!(seq(&mut ctl, &mut f, 20));
    assert_eq!(state(&ctl, 20), 0);
    assert!(!ctl.record(20).unwrap().has_callback(event::NPC_DEACTIVATE));
    // An absent chain: 0.
    ctl.records.retain(|r| r.chain != 20);
    assert!(!seq(&mut ctl, &mut f, 20));
    assert!(f.log().is_empty());
}

// ------------------------------------------------------------ §2

// Covers: specs/world/quests-act3.md §2
#[test]
fn init_stores() {
    let (ctl, _) = control();
    for chain in [14, 15, 16, 17, 18, 19, 20, 28, 39] {
        assert!(ctl.record(chain).unwrap().active, "chain {chain}");
    }
    let r = |c| ctl.record(c).unwrap();
    // State, init_no, seq_id.
    assert_eq!((r(15).state, r(15).init_no, r(15).seq_id), (0, 4, Some(19)));
    assert_eq!((r(16).init_no, r(16).seq_id), (0, Some(15)));
    assert_eq!((r(17).init_no, r(17).seq_id), (4, Some(15)));
    assert_eq!((r(18).init_no, r(18).seq_id), (0, Some(16)));
    assert_eq!((r(19).init_no, r(19).seq_id), (6, Some(20)));
    assert_eq!((r(20).init_no, r(20).seq_id), (6, None));
    // Chain 15 +0x00 := 1.
    assert!(r(15).extra.act3.q1.reward_open);
    // Chain 16 status := 1; the others 0.
    assert_eq!(r(16).status, 1);
    for c in [14, 15, 17, 18, 19, 20, 28] {
        assert_eq!(r(c).status, 0, "chain {c}");
    }
    // Chain 18 +0x01, +0x0C := 1.
    assert!(r(18).extra.act3.q4.may_choose && r(18).extra.act3.q4.to_drop);
    // Chain 28 +0x01 := 1; it reports in slot 16.
    assert!(r(28).extra.act3.q7.to_spawn);
    assert_eq!(r(28).filter, 16);
    // Everything else zeroed.
    let q1 = act3::q1::Extra {
        reward_open: true,
        ..Default::default()
    };
    assert_eq!(r(15).extra.act3.q1, q1);
    assert_eq!(r(14).extra.act3.q0, act3::q0::Extra::default());
    let q7 = act3::q7::Extra {
        to_spawn: true,
        ..Default::default()
    };
    assert_eq!(r(28).extra.act3.q7, q7);
}

/// The 0x5D recipients and their chain / flags bytes.
fn recipients(f: &Fake3) -> Vec<(UnitId, u8, u8)> {
    sent_5d(f).iter().map(|m| (m.0, m.1[1], m.1[2])).collect()
}

// Covers: specs/world/quests-act3.md §2, §1.1
#[test]
fn status_iterate_rule() {
    const P3: UnitId = UnitId(3);
    const P4: UnitId = UnitId(4);
    const P5: UnitId = UnitId(5);
    for chain in [15, 16, 17, 18, 19, 20] {
        let (mut ctl, _) = control();
        let mut f = Fake3::new();
        let slot = ctl.record(chain).unwrap().filter;
        assert_eq!(slot, chain + 2);
        for u in [P2, P3, P4, P5] {
            f.add_player(u, 75);
        }
        // P1: nothing. P2: s.0. P3: s.0, s.13. P4: s.15, s.14. P5: s.0
        // but not in Act III.
        let set = |f: &mut Fake3, u, bits: &[u8]| {
            for &b in bits {
                f.p(u).quests.flags[0].set(slot, b);
            }
        };
        set(&mut f, P2, &[bit::REWARD_GRANTED]);
        set(&mut f, P3, &[bit::REWARD_GRANTED, bit::PRIMARY_GOAL_DONE]);
        set(&mut f, P4, &[bit::COMPLETED_BEFORE, bit::COMPLETED_NOW]);
        f.p(P5).act = Some(0);
        let i = ctl.find(chain).unwrap();
        ctl.records[i].flags = 0x55;
        act3::status_all(&mut ctl, &mut f, i, 3);
        assert_eq!((ctl.records[i].flags, ctl.records[i].status), (0, 3));
        let want: Vec<(UnitId, u8, u8)> = if chain == 16 {
            // Only 18.0 and 18.15 both clear.
            vec![(P1, 16, 0)]
        } else {
            vec![(P1, chain, 0), (P3, chain, 0), (P4, chain, 0)]
        };
        assert_eq!(recipients(&f), want, "chain {chain}");

        // Silent: the same store, nothing sent.
        f.f.sent.clear();
        ctl.records[i].flags = 0x55;
        act3::status_silent(&mut ctl, i, 4);
        assert_eq!((ctl.records[i].flags, ctl.records[i].status), (0, 4));
        assert!(f.f.sent.is_empty());
    }
}

// ------------------------------------------------------------ §10

// Covers: specs/world/quests-act3.md §10
#[test]
fn creation_links() {
    assert_eq!(act3::monster_link(407, 407), Some(17));
    assert_eq!(act3::monster_link(366, 366), Some(19));
    assert_eq!(act3::monster_link(242, 242), Some(20));
    assert_eq!(act3::monster_link(242, 704), None);
    for base in [0, 245, 253, 297, 368, 406, 408] {
        assert_eq!(act3::monster_link(base, base), None);
    }
    for s in [26, 27, 29] {
        assert_eq!(act3::superunique_link(s), Some(19));
    }
    for s in [0, 25, 28, 30, 36] {
        assert_eq!(act3::superunique_link(s), None);
    }
}

// ------------------------------------------------------------ dispatch

// Covers: specs/world/quests-act3.md §1.3, §2
#[test]
fn dispatch_reaches_act3() {
    // Game entry calls chain 18's sequence function: no unhandled report.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    ctl.player_enters(&mut f, P1, 0).unwrap();
    assert!(!f.log().iter().any(|l| l.starts_with("unhandled 18 ")));
    assert_eq!(state(&ctl, 18), 0);
    assert_eq!((state(&ctl, 16), state(&ctl, 17)), (0, 0));
    for c in [14, 28] {
        let p = format!("unhandled {c} ");
        assert!(!f.log().iter().any(|l| l.starts_with(&p)), "chain {c}");
    }

    // A player who finished the Golden Bird before: chain 18 not-intro
    // 0, the walk opens Khalim's Will and the Blade.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.p(P1).quests.flags[0].set(20, bit::COMPLETED_BEFORE);
    ctl.player_enters(&mut f, P1, 0).unwrap();
    assert!(!ctl.record(18).unwrap().not_intro);
    assert_eq!((state(&ctl, 16), state(&ctl, 17)), (1, 1));
    assert!(!f.log().iter().any(|l| l.starts_with("unhandled 18 ")));

    // Status functions of chains 14, 28, 39 return false, unreported.
    let (ctl, _) = control();
    let mut f = Fake3::new();
    let pf = f.flags(P1);
    for c in [14, 28, 39] {
        let r = ctl.record(c).unwrap();
        let i = ctl.find(c).unwrap();
        let sf = r.status_fn.unwrap_or(0);
        assert_eq!(act1::status_fn(&ctl, &mut f, i, P1, &pf, sf), None);
    }
    assert!(f.log().is_empty());
}
