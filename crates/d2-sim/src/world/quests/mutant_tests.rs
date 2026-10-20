// Spec: specs/world/quests.md §2.2, §4, §5; specs/world/quests-act1.md §10.4, §10.8
//! Mutation-testing kills (METHODS M08): each test pins an outcome the
//! spec decides that no earlier test checked.

use super::tests::*;
use super::*;

// From specs/world/quests.md §2.2: clearing one callback keeps the others.
#[test]
fn clear_callback_clears_one_bit() {
    let (ctl, _) = control();
    let mut r = ctl.record(1).unwrap().clone();
    r.callbacks = 0b1111;
    r.clear_callback(1);
    assert_eq!(r.callbacks, 0b1101);
    assert!(r.has_callback(0) && !r.has_callback(1) && r.has_callback(3));
}

// From specs/world/quests.md §5 r1 (edge case 6): at the wrap every due
// becomes 0xFFFFFFFF − due.
#[test]
fn timer_wrap_subtracts_due() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.tick = u32::MAX - 1;
    for due in [0, 1] {
        ctl.timers.push(QuestTimer {
            func: TimerFn::Probe,
            chain: 9,
            due,
            period: 3,
        });
    }
    ctl.update(&mut f);
    // due 0 → 0xFFFFFFFF: not < tick, kept; due 1 → 0xFFFFFFFE: fires.
    assert_eq!(f.log, ["unhandled 9 0xffffffff"]);
    assert_eq!(ctl.timers[0].due, u32::MAX);
    assert_eq!(ctl.timers[1].due, 2);
}

// From specs/world/quests.md §4.6: a monster is linked to its level's
// quest chain unless the level's `Quest` is 0.
#[test]
fn link_monster_uses_the_level_quest() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let (a, b) = (UnitId(0x40), UnitId(0x41));
    f.chains.insert(a, QuestChain::default());
    f.chains.insert(b, QuestChain::default());
    ctl.link_monster(&mut f, a, 1);
    ctl.link_monster(&mut f, b, 0);
    assert_eq!(f.chains[&a].0, [1]);
    assert!(f.chains[&b].0.is_empty());
}

// From specs/world/quests.md §4.4: classes 242, 243, 391, 544 force the
// dispatch without a superunique.
#[test]
fn forced_class_without_superunique() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let victim = UnitId(0x40);
    f.chains.insert(victim, QuestChain(vec![6]));
    f.monsters.insert(
        victim,
        (
            0x40,
            242,
            UnitKind::Monster {
                class: 242,
                superunique: None,
                owner: None,
            },
        ),
    );
    ctl.monster_killed(&mut f, victim, Some(P1));
    assert_eq!(f.log.iter().filter(|l| l.starts_with("drop")).count(), 3);
}

// From specs/world/quests-act1.md §10.4: event 3 leaving level 1 while state 2
// sets state 3 (the old level travels in the first argument).
#[test]
fn den_leaving_town_uses_the_old_level() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.record_mut(1).unwrap().state = 2;
    ctl.changed_level(&mut f, P1, 1, 2);
    assert_eq!(ctl.record(1).unwrap().state, 3);
}

// From specs/world/quests.md §4.2: NPC chat end (event 2) is filtered by
// the player's act.
#[test]
fn chat_end_is_filtered_by_the_players_act() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.player_enters(&mut f, P1, 0).unwrap();
    let msg = hex("31 10000000 4000 0000");
    ctl.quest_message(&mut f, P1, &msg[..9]);
    assert_eq!(ctl.record(1).unwrap().state, 2);
    f.sent.clear();
    f.p(P1).act = Some(1);
    ctl.npc_deactivate(&mut f, P1, AKARA_U);
    assert!(f.sent.is_empty());
    f.p(P1).act = Some(0);
    ctl.npc_deactivate(&mut f, P1, AKARA_U);
    assert!(!f.sent.is_empty());
}

// From specs/world/quests.md §6.1 r1, r2.
#[test]
fn default_status_remaining_branches() {
    let none = QuestFlags::default();
    // s < n, not now, L = 6: only chain 4 (not done) gives 12.
    let r = rec_with(1, 2, 4, 6);
    assert_eq!(QuestControl::default_status(&r, &none), Ok(6));
    let mut done4 = QuestFlags::default();
    done4.set(4, bit::PRIMARY_GOAL_DONE);
    let r = rec_with(4, 2, 6, 6);
    assert_eq!(QuestControl::default_status(&r, &done4), Ok(6));
    let r = rec_with(4, 2, 6, 5);
    assert_eq!(QuestControl::default_status(&r, &none), Ok(5));
    // s ≥ n, done, q = 40: L (the assert is q ≤ 40).
    let mut r = rec_with(37, 1, 0, 3);
    r.filter = 40;
    let mut d40 = QuestFlags::default();
    d40.set(40, bit::PRIMARY_GOAL_DONE);
    assert_eq!(QuestControl::default_status(&r, &d40), Ok(3));
    // s ≥ n, not done: chain 4 without now → 12; now on another chain → 12.
    let r = rec_with(4, 5, 4, 3);
    assert_eq!(QuestControl::default_status(&r, &none), Ok(12));
    let mut now1 = QuestFlags::default();
    now1.set(1, bit::COMPLETED_NOW);
    let r = rec_with(1, 5, 4, 3);
    assert_eq!(QuestControl::default_status(&r, &now1), Ok(12));
}

// From specs/world/quests.md §6.2 r2: the assert is chain ≤ 40.
#[test]
fn quest_data_accepts_chain_40() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.record_mut(40).unwrap().status = 1;
    assert_eq!(ctl.request_quest_data(&mut f, P1), Ok(()));
    assert_eq!(f.sent_ids().last(), Some(&0x52));
}

// From specs/world/quests.md §6.2 r3: barbarians are read only when
// list[36] ≠ 0.
#[test]
fn quest_data_reads_barbarians_only_for_list_36() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.record_mut(1).unwrap().status = 1;
    ctl.request_quest_data(&mut f, P1).unwrap();
    let m = f.sent.iter().find(|m| m.1[0] == 0x50).unwrap();
    // Barbarians left would be 15 (no cage spawned, quests-act5.md
    // §4.1); not read, the field stays 0.
    assert_eq!(m.1[7..9], [0, 0]);
    assert!(!f.log.iter().any(|l| l.starts_with("unhandled 32")));
}

// From specs/world/quests.md §6.3: filter 36's extra is the barbarians
// left.
#[test]
fn status_message_filter_36_reads_barbarians() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let r = ctl.records.iter().find(|r| r.filter == 36).unwrap();
    let (chain, act) = (r.chain, r.act);
    f.p(P1).act = Some(act);
    ctl.record_mut(chain).unwrap().extra.a5.q2.cage_spawned[0] = true;
    ctl.send_status(&mut f, P1, chain).unwrap();
    // `0x00588C50`: 5 per cage group not spawned (two here).
    assert_eq!(f.sent.last().unwrap().1[4..6], [10, 0]);
}

// From specs/world/quests.md §8.1, specs/world/quests-act1.md §10.8: Warriv calls chain 6's callback
// 3 (a = 1, b = 40: state 4 → 5) only when slot 6 bits 13 and 0 are set
// and the record is not-intro.
#[test]
fn warriv_chain6_callback_needs_all_three() {
    let call = |bits: &[u8], not_intro: bool| {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        for &b in bits {
            f.p(P1).quests.flags[0].set(6, b);
        }
        ctl.record_mut(6).unwrap().not_intro = not_intro;
        ctl.record_mut(6).unwrap().state = 4;
        ctl.act_completion(&mut f, P1, npc::WARRIV1).unwrap();
        ctl.record(6).unwrap().state == 5
    };
    assert!(call(&[0, 13], true));
    assert!(!call(&[], true));
    assert!(!call(&[13], false));
    assert!(!call(&[0], true));
}

// From specs/world/quests.md §8.1: Tyrael (expansion) sets 28.0 and 28.13;
// `5D 17 02 00 0000` then `61 05` only when +0x4C ≠ 1.
#[test]
fn tyrael_act_completion() {
    for (byte4c, step) in [(0, true), (1, false)] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        f.p(P1).byte4c = byte4c;
        ctl.act_completion(&mut f, P1, npc::TYRAEL2).unwrap();
        let fl = f.flags(P1);
        assert!(fl.get(28, 0) && fl.get(28, 13));
        let want: Vec<Vec<u8>> = if step {
            vec![hex("5d 17 02 00 0000"), hex("61 05")]
        } else {
            vec![]
        };
        let got: Vec<Vec<u8>> = f.sent[1..].iter().map(|m| m.1.clone()).collect();
        assert_eq!(f.sent[0].1[0], 0x28);
        assert_eq!(got, want, "+0x4C {byte4c}");
    }
}

// From specs/world/quests.md §6.7: act 4 (V) sends 0x91; acts past V do
// not exist and send nothing.
#[test]
fn npc_gossip_act_v() {
    let mut f = Fake::new();
    let n = INTRO_NPCS[4][0];
    f.p(P1).quests.intro[0].insert(n);
    npc_gossip(&mut f, P1, 4);
    assert_eq!(f.sent.len(), 1);
    assert_eq!(f.sent[0].1[..4], [0x91, 4, n as u8, (n >> 8) as u8]);
    f.sent.clear();
    npc_gossip(&mut f, P1, 5);
    assert!(f.sent.is_empty());
}

// From specs/world/quests.md §8.2.
#[test]
fn warp_check_100_and_132() {
    assert_eq!(warp_check(1, 100), WarpCheck::Delegate(0x005B_BFA0));
    assert_eq!(warp_check(1, 132), WarpCheck::Delegate(0x0058_E640));
}

// From specs/world/quests.md §9.4: `bkd ` sends the stone order, `trs `
// the true tomb (level 68 − 66 = 2, kept at chain 13's extra +0x34).
#[test]
fn read_clue_by_code() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    read_clue(&mut ctl, &mut f, P1, *b"trs ");
    assert_eq!(f.sent, [(P1, hex("50 0d00 0200 00000000000000000000"))]);
    assert_eq!(ctl.record(13).unwrap().extra.tomb_level, 68);
    f.sent.clear();
    read_clue(&mut ctl, &mut f, P1, *b"bkd ");
    assert_eq!(f.sent.len(), 1);
    assert_eq!(f.sent[0].1[..3], [0x50, 4, 0]);
    f.sent.clear();
    read_clue(&mut ctl, &mut f, P1, *b"xyz ");
    assert!(f.sent.is_empty() && f.log.is_empty());
}

// ------------------------------------------------------------ Act I (§10)

fn call(ctl: &mut QuestControl, f: &mut Fake, chain: u8, args: EventArgs) {
    let i = ctl.find(chain).unwrap();
    act1::callback(ctl, f, i, args, None, false);
}

/// Event 11 (scroll message `msg` from NPC class `npc`) to `chain`.
fn scroll(ctl: &mut QuestControl, f: &mut Fake, chain: u8, npc_class: u16, msg: u32) {
    let args = EventArgs {
        event: event::SCROLL_MESSAGE,
        player: Some(P1),
        a: u32::from(npc_class),
        b: msg,
        ..EventArgs::default()
    };
    call(ctl, f, chain, args);
}

// From specs/world/quests-act1.md §10.2, §10.5–§10.8: each start message counts
// only from the quest's NPC.
#[test]
fn start_messages_need_the_quest_npc() {
    for (chain, msg, right, wrong) in [
        (2, 81, npc::KASHYA, npc::AKARA),
        (3, 146, npc::CHARSI, npc::AKARA),
        (4, 97, npc::AKARA, npc::KASHYA),
        (6, 166, npc::CAIN5, npc::AKARA),
    ] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        let before = ctl.record(chain).unwrap().state;
        scroll(&mut ctl, &mut f, chain, wrong, msg);
        assert_eq!(ctl.record(chain).unwrap().state, before, "chain {chain}");
        scroll(&mut ctl, &mut f, chain, right, msg);
        assert_eq!(ctl.record(chain).unwrap().state, 2, "chain {chain}");
    }
}

// From specs/world/quests-act1.md §10.1: the start sets bit 2 only for players
// without bit 0 or 1.
#[test]
fn start_skips_players_with_reward_bits() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.players.insert(
        P2,
        Player {
            guid: 2,
            act: Some(0),
            ..Player::default()
        },
    );
    f.p(P1).quests.flags[0].set(2, bit::REWARD_GRANTED);
    scroll(&mut ctl, &mut f, 2, npc::KASHYA, 81);
    assert!(!f.flags(P1).get(2, bit::STARTED));
    assert!(f.flags(P2).get(2, bit::STARTED));
}

// From specs/world/quests-act1.md §10.3: Warriv's message 0 or 1 sets 0.0.
#[test]
fn warriv_gossip_message_needs_warriv_and_0_or_1() {
    for (npc_class, msg, set) in [
        (npc::AKARA, 0, false),
        (npc::WARRIV1, 2, false),
        (npc::WARRIV1, 1, true),
    ] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        scroll(&mut ctl, &mut f, 0, npc_class, msg);
        assert_eq!(
            f.flags(P1).get(0, bit::REWARD_GRANTED),
            set,
            "{npc_class} {msg}"
        );
    }
}

// From specs/world/quests-act1.md §10.4–§10.8: reward messages need the NPC
// and the player's bit 1 (or the item).
#[test]
fn reward_messages_need_their_condition() {
    // Den 76 without 1.1: nothing.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let before = ctl.record(1).unwrap().state;
    scroll(&mut ctl, &mut f, 1, npc::AKARA, 76);
    assert_eq!(ctl.record(1).unwrap().state, before);
    assert_eq!(f.flags(P1), QuestFlags::default());
    // Kashya 92: needs Kashya and 2.1.
    for (npc_class, pending, merc) in [
        (npc::KASHYA, false, false),
        (npc::AKARA, true, false),
        (npc::KASHYA, true, true),
    ] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        if pending {
            f.p(P1).quests.flags[0].set(2, bit::REWARD_PENDING);
            f.p(P1).quests.flags[0].set(2, bit::PRIMARY_GOAL_DONE);
        }
        scroll(&mut ctl, &mut f, 2, npc_class, 92);
        assert_eq!(f.log.iter().any(|l| l == "merc 150"), merc);
        assert_eq!(ctl.record(2).unwrap().state == 5, merc);
    }
    // Akara 112 needs `bks `.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    scroll(&mut ctl, &mut f, 4, npc::AKARA, 112);
    assert!(f.log.is_empty());
    assert_ne!(ctl.record(4).unwrap().state, 5);
    // 118 needs 4.1.
    scroll(&mut ctl, &mut f, 4, npc::AKARA, 118);
    assert!(f.log.is_empty() && f.sent.is_empty());
    // Warriv 183: needs Warriv and 6.1.
    for (npc_class, pending, granted) in [
        (npc::WARRIV1, false, false),
        (npc::AKARA, true, false),
        (npc::WARRIV1, true, true),
    ] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        if pending {
            f.p(P1).quests.flags[0].set(6, bit::REWARD_PENDING);
        }
        scroll(&mut ctl, &mut f, 6, npc_class, 183);
        assert_eq!(f.flags(P1).get(6, bit::REWARD_GRANTED), granted);
    }
}

// From specs/world/quests-act1.md §10.8: Akara 179, Kashya 181, Cain 184 remove
// the player from that NPC's list; other NPCs do not.
#[test]
fn chain6_list_removal_by_npc() {
    let lists = |ctl: &QuestControl| {
        let x = &ctl.record(6).unwrap().extra.q6;
        [
            x.akara.contains(1),
            x.kashya.contains(1),
            x.cain.contains(1),
        ]
    };
    for (k, msg, right, wrong) in [
        (0, 179, npc::AKARA, npc::KASHYA),
        (1, 181, npc::KASHYA, npc::AKARA),
        (2, 184, npc::CAIN5, npc::AKARA),
    ] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        let x = &mut ctl.record_mut(6).unwrap().extra.q6;
        x.akara.add(1);
        x.kashya.add(1);
        x.cain.add(1);
        scroll(&mut ctl, &mut f, 6, wrong, msg);
        assert_eq!(lists(&ctl), [true; 3], "{msg}");
        scroll(&mut ctl, &mut f, 6, right, msg);
        let mut want = [true; 3];
        want[k] = false;
        assert_eq!(lists(&ctl), want, "{msg}");
    }
}

// From specs/world/quests-act1.md §10.7: town messages 140–145 finish the
// tower quest (state 5) once, for a player with 5.13 after the kill.
#[test]
fn tower_town_messages_complete() {
    for msg in [140, 145] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        ctl.record_mut(5).unwrap().state = 4;
        ctl.record_mut(5).unwrap().extra.q5.report_due = true;
        f.p(P1).quests.flags[0].set(5, bit::PRIMARY_GOAL_DONE);
        scroll(&mut ctl, &mut f, 5, npc::AKARA, msg);
        assert_eq!(ctl.record(5).unwrap().state, 5);
        assert!(!ctl.record(5).unwrap().extra.q5.report_due);
    }
    // Without 5.13: nothing.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.record_mut(5).unwrap().state = 4;
    ctl.record_mut(5).unwrap().extra.q5.report_due = true;
    scroll(&mut ctl, &mut f, 5, npc::AKARA, 140);
    assert_eq!(ctl.record(5).unwrap().state, 4);
}

/// Event 3 to chain `chain` in `state` with the player's slot bits.
fn area(chain: u8, state: u8, bits: &[u8], old: u32, new: u32) -> (u8, QuestFlags) {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    for &b in bits {
        f.p(P1).quests.flags[0].set(chain, b);
    }
    ctl.record_mut(chain).unwrap().state = state;
    let args = EventArgs {
        event: event::CHANGED_LEVEL,
        player: Some(P1),
        a: old,
        b: new,
        ..EventArgs::default()
    };
    call(&mut ctl, &mut f, chain, args);
    (ctl.record(chain).unwrap().state, f.flags(P1))
}

// From specs/world/quests-act1.md §10.4, §10.5: entering the area from state 1
// or 2 → state 3 and, through I2 after status 2, bit 4 for every player
// with neither 0 nor 1; leaving level 1 in state 2 while the status is
// still 0 → state 3 and bit 4 (I2 before the status, bug kept).
#[test]
fn area_event_states_and_bits() {
    use bit::{ENTER_AREA as EA, LEAVE_TOWN as LT, REWARD_GRANTED as G, STARTED as S};
    let (s, fl) = area(1, 1, &[S], 2, 8);
    assert!(s == 3 && fl.get(1, EA));
    let (s, fl) = area(1, 1, &[], 2, 8);
    assert!(s == 3 && fl.get(1, EA));
    let (_, fl) = area(1, 1, &[S, G], 2, 8);
    assert!(!fl.get(1, EA));
    let (_, fl) = area(1, 1, &[S, bit::REWARD_PENDING], 2, 8);
    assert!(!fl.get(1, EA));
    let (s, fl) = area(1, 2, &[S], 1, 2);
    assert!(s == 3 && fl.get(1, EA) && !fl.get(1, LT));
    assert_eq!(area(1, 4, &[S], 2, 8).0, 4);
    assert_eq!(area(1, 1, &[S], 1, 2).0, 1);
    assert_eq!(area(1, 2, &[S], 5, 2).0, 2);
    // Chain 2's area: the Burial Grounds (17), from state 0 too.
    assert_eq!(area(2, 1, &[S], 2, 17).0, 3);
    assert_eq!(area(2, 0, &[], 2, 17).0, 3);
    assert_eq!(area(2, 3, &[], 2, 17).0, 3);
}

// From specs/world/quests-act1.md §10.3: chain 25's callback 8 is a bare `ret`.
#[test]
fn flavie_kill_callback_does_nothing() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let args = EventArgs {
        event: event::MONSTER_KILLED,
        player: Some(P1),
        target: Some(UnitId(0x40)),
        ..EventArgs::default()
    };
    call(&mut ctl, &mut f, 25, args);
    assert!(f.log.is_empty() && f.sent.is_empty());
}

// From specs/world/quests-act1.md §10.5: Blood Raven's kill sets 2.13 and 2.1
// for players near her, sound 34, state 4.
#[test]
fn blood_raven_kill() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.near = vec![P1];
    let kind = UnitKind::Monster {
        class: 267,
        superunique: Some(0),
        owner: None,
    };
    f.monsters.insert(UnitId(0x40), (0x40, 267, kind));
    let args = EventArgs {
        event: event::MONSTER_KILLED,
        player: Some(P1),
        target: Some(UnitId(0x40)),
        ..EventArgs::default()
    };
    call(&mut ctl, &mut f, 2, args);
    let fl = f.flags(P1);
    assert!(fl.get(2, bit::PRIMARY_GOAL_DONE) && fl.get(2, bit::REWARD_PENDING));
    assert_eq!(ctl.record(2).unwrap().state, 4);
}

// From specs/world/quests-act1.md §10.6: the Cow King's death (killer with
// 40.0 in an expansion game) sets 4.10 for players in level 39 and drops 8
// `vps `.
#[test]
fn cow_king_kill() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).level = Some(39);
    f.p(P1).quests.flags[0].set(40, bit::REWARD_GRANTED);
    let args = EventArgs {
        event: event::MONSTER_KILLED,
        player: Some(P1),
        target: Some(UnitId(0x40)),
        ..EventArgs::default()
    };
    call(&mut ctl, &mut f, 4, args);
    assert!(f.flags(P1).get(4, 10));
    assert_eq!(
        f.log.iter().filter(|l| l.starts_with("drop vps")).count(),
        8
    );
}

// From specs/world/quests.md §6.1 and §6.2 r2: every status function
// `quests.tsv` registers has a body now (Act II's chains 26 and 27 return
// false and write nothing, quests-act2-2.md §1 item 18); a function with
// no body on any chain is reported and nothing is written for it.
#[test]
fn unspecified_status_function_is_reported() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    for r in &mut ctl.records {
        r.status = u8::from(r.status_fn.is_some());
    }
    ctl.request_quest_data(&mut f, P1).unwrap();
    assert!(f.log.is_empty(), "{:?}", f.log);
    let list = &f.sent.last().unwrap().1;
    assert_eq!((list[1 + 30], list[1 + 31]), (0, 0));
    // Chain 1 has no status function in 1.14d; one with no body is
    // reported (the fallback arm) and nothing is written for it.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    for r in &mut ctl.records {
        r.status = u8::from(r.chain == 1);
    }
    ctl.record_mut(1).unwrap().status_fn = Some(0xDEAD);
    ctl.request_quest_data(&mut f, P1).unwrap();
    assert_eq!(f.log, ["unhandled 1 0xdead"]);
    let list = &f.sent.last().unwrap().1;
    assert!(list[1..].iter().all(|&b| b == 0));
}

// From specs/world/quests-act1.md §10.4: message 76 runs the state-5 step only
// with 1.13; the grant always follows.
#[test]
fn den_reward_without_goal_bit() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).quests.flags[0].set(1, bit::REWARD_PENDING);
    let (state, status) = (ctl.record(1).unwrap().state, ctl.record(1).unwrap().status);
    scroll(&mut ctl, &mut f, 1, npc::AKARA, 76);
    assert_eq!(ctl.record(1).unwrap().state, state);
    assert_eq!(ctl.record(1).unwrap().status, status);
    assert!(f.flags(P1).get(1, bit::REWARD_GRANTED));
}

// From specs/world/quests-act1.md §10.4: "rooms ≤ room count and monsters left
// < 6, or status = 4 and monsters left > 5" → flags 0x20.
#[test]
fn den_kill_few_left_conditions() {
    // (spawn, kills, rooms, populated), status, fires
    let cases = [
        ((10, 7, 5, 6), 0, false), // too many rooms, 3 left
        ((10, 4, 5, 5), 0, false), // 6 left
        ((10, 4, 5, 6), 0, false), // too many rooms, 6 left, status 0
        ((10, 5, 5, 6), 4, false), // status 4, 5 left
        ((10, 4, 5, 6), 4, true),  // status 4, 6 left
    ];
    for (den, status, fires) in cases {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        let victim = UnitId(0x40);
        f.chains.insert(victim, QuestChain(vec![1]));
        f.den = den;
        ctl.record_mut(1).unwrap().status = status;
        ctl.record_mut(1).unwrap().flags = 0;
        ctl.monster_killed(&mut f, victim, Some(P1));
        assert_eq!(
            ctl.record(1).unwrap().flags == 0x20,
            fires,
            "{den:?} {status}"
        );
    }
}

// From specs/world/quests-act1.md §10.5: Blood Raven's kill skips players with
// 2.0 or 2.1.
#[test]
fn blood_raven_kill_skips_rewarded_players() {
    for b in [bit::REWARD_GRANTED, bit::REWARD_PENDING] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        f.near = vec![P1];
        f.p(P1).quests.flags[0].set(2, b);
        let args = EventArgs {
            event: event::MONSTER_KILLED,
            player: Some(P1),
            target: Some(UnitId(0x40)),
            ..EventArgs::default()
        };
        call(&mut ctl, &mut f, 2, args);
        assert!(!f.flags(P1).get(2, bit::PRIMARY_GOAL_DONE), "bit {b}");
        assert!(!f.log.iter().any(|l| l.starts_with("sound")), "bit {b}");
    }
}

// From specs/world/quests-act1.md §10.6: the stone order is drawn on the quest
// seed once and kept; reading the deciphered scroll computes it.
#[test]
fn stone_order_draws_on_the_quest_seed() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let mut s = ctl.seed;
    let want = act1::stone_order_from(|| s.step());
    read_clue(&mut ctl, &mut f, P1, *b"bkd ");
    assert_eq!(ctl.record(4).unwrap().extra.stone_order, Some(want));
    assert_eq!(ctl.seed, s);
    assert_eq!(act1::stone_order(&mut ctl), want);
}

// From specs/world/quests-act1.md §10.6: a run with no piles left drops nothing.
#[test]
fn wirt_body_with_no_piles() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.record_mut(4).unwrap().extra.wirt_piles = Some(0);
    object_event(&mut ctl, &mut f, UnitId(0x70), act1::WIRT_BODY);
    assert!(f.log.is_empty());
    assert_eq!(ctl.record(4).unwrap().extra.wirt_piles, Some(0));
}

// From specs/world/quests-act1.md §10.5: the Malus drops only for a player with
// neither 3.0 nor 3.1.
#[test]
fn malus_skips_rewarded_players() {
    for b in [bit::REWARD_GRANTED, bit::REWARD_PENDING] {
        let (mut ctl, _) = control();
        let mut f = Fake::new();
        f.p(P1).stats.insert(12, 8);
        f.p(P1).quests.flags[0].set(3, b);
        act1::malus_operate(&mut ctl, &mut f, UnitId(0x71), P1);
        assert!(f.log.is_empty(), "bit {b}");
    }
}

// From specs/world/quests-act1.md §10.5: the imbue grant sets 3.0, clears 3.1.
#[test]
fn imbue_grant_bits() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.p(P1).quests.flags[0].set(3, bit::REWARD_PENDING);
    act1::imbue_granted(&mut ctl, &mut f, P1);
    let fl = f.flags(P1);
    assert!(fl.get(3, bit::REWARD_GRANTED) && !fl.get(3, bit::REWARD_PENDING));
}

// From specs/world/quests.md §7.1: the entries of one table, state and NPC.
#[test]
fn messages_for_filters_all_three() {
    let t = QuestTables::load().unwrap();
    for m0 in &t.messages {
        let got: Vec<_> = t.messages_for(m0.table, m0.state, m0.npc).collect();
        let want: Vec<_> = t
            .messages
            .iter()
            .filter(|m| m.table == m0.table && m.state == m0.state && m.npc == m0.npc)
            .collect();
        assert_eq!(got, want);
    }
}
