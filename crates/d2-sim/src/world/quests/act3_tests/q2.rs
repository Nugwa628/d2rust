// Spec: specs/world/quests-act3.md §4 (A3Q2 Khalim's Will, chain 16)
//! Chain 16 callback by callback, the status and active functions,
//! Khalim's chests, the sewer lever and stairs and the cube hook.

use super::*;
use crate::rng::Seed;

const C: u8 = 16;
const S: u8 = 18;
const P3: UnitId = UnitId(3);
const ITEM_U: UnitId = UnitId(0x300);
const CHEST_U: UnitId = UnitId(0x70);
const LEVER_U: UnitId = UnitId(0x71);
const STAIRS_U: UnitId = UnitId(0x72);

fn setf(f: &mut Fake3, u: UnitId, slot: u8, b: u8) {
    f.p(u).quests.flags[0].set(slot, b);
}

fn idx(ctl: &QuestControl) -> usize {
    ctl.find(C).unwrap()
}

fn x(ctl: &QuestControl) -> &act3::q2::Extra {
    &ctl.record(C).unwrap().extra.act3.q2
}

fn ev(e: u8, p: UnitId, target: Option<UnitId>, a: u32, b: u32) -> EventArgs {
    EventArgs {
        event: e,
        target,
        player: Some(p),
        a,
        b,
    }
}

fn give(f: &mut Fake3, p: UnitId, codes: &[&[u8; 4]]) {
    for c in codes {
        f.p(p).items.push(**c);
    }
}

fn cain(ctl: &mut QuestControl, f: &mut Fake3, p: UnitId) -> TextList {
    text(ctl, f, C, p, CAIN3_U)
}

fn status_of(ctl: &QuestControl, f: &mut Fake3, p: UnitId) -> u8 {
    let pf = f.flags(p);
    act3::status(ctl, f, idx(ctl), p, &pf).unwrap()
}

fn active(ctl: &QuestControl, f: &mut Fake3, p: UnitId) -> bool {
    act3::active(ctl, f, idx(ctl), p, act3::npc::CAIN3)
}

fn smash(ctl: &mut QuestControl) {
    ctl.record_mut(19).unwrap().extra.act3.q5.orb_smashed = true;
}

fn no_unhandled(f: &Fake3) {
    assert!(
        !f.log().iter().any(|l| l.starts_with("unhandled 16 ")),
        "{:?}",
        f.log()
    );
}

/// Pick up an item of `c` as `p`.
fn pick(ctl: &mut QuestControl, f: &mut Fake3, p: UnitId, c: &[u8; 4]) {
    f.f.item_codes.insert(ITEM_U, *c);
    call(ctl, f, C, ev(event::ITEM_PICKED_UP, p, Some(ITEM_U), 0, 0));
}

// Covers: specs/world/quests-act3.md §4.3 text, §4.3 r1, §1.2
#[test]
fn chat_start() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    // State 0, nothing held, 18.2 clear: nothing.
    assert!(cain(&mut ctl, &mut f, P1).is_empty());
    // State ≠ 0 and 18.2 clear → table state 0 (msg 543), even holding
    // parts.
    ctl.records[i].state = 1;
    give(&mut f, P1, &[b"qey "]);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(543, 0)]);
    // Only Cain.
    assert!(text(&mut ctl, &mut f, C, P1, ALKOR_U).is_empty());
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §4.3 r2, §4.1
#[test]
fn chat_untold_parts() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    setf(&mut f, P1, S, 2);
    // The Will (orb intact) wins over everything.
    give(&mut f, P1, &[b"qbr ", b"qhr ", b"qey ", b"qf1 ", b"qf2 "]);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(548, 0)]);
    // Orb smashed: the Will is skipped; the flail next.
    let (mut ctl2, _) = control();
    smash(&mut ctl2);
    assert_eq!(cain(&mut ctl2, &mut f, P1), vec![(547, 0)]);
    setf(&mut f, P1, S, 7);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(547, 0)]);
    setf(&mut f, P1, S, 5);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(545, 0)]);
    setf(&mut f, P1, S, 3);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(544, 0)]);
    setf(&mut f, P1, S, 6);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(546, 0)]);
    // All told: step 3, flail held → 10.
    setf(&mut f, P1, S, 4);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(547, 2)]);
}

// Covers: specs/world/quests-act3.md §4.3 r3
#[test]
fn chat_parts_held() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    for b in [2, 3, 4, 5, 6, 7] {
        setf(&mut f, P1, S, b);
    }
    // n = 0: 18.2 → 6.
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(543, 2)]);
    // n = 0 with the Will (told) → 11.
    give(&mut f, P1, &[b"qf2 "]);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(548, 2)]);
    // Brain only → 9; brain + eye → 7; + heart → 8; + flail → 10.
    give(&mut f, P1, &[b"qbr "]);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(546, 2)]);
    give(&mut f, P1, &[b"qey "]);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(545, 2)]);
    give(&mut f, P1, &[b"qhr "]);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(544, 2)]);
    give(&mut f, P1, &[b"qf1 "]);
    assert_eq!(cain(&mut ctl, &mut f, P1), vec![(547, 2)]);
    // n = 0, no Will, 18.2 clear: nothing.
    let mut g = Fake3::new();
    assert!(cain(&mut ctl, &mut g, P1).is_empty());
}

// Covers: specs/world/quests-act3.md §4.3 r3
#[test]
fn wants_to_talk() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    assert!(!active(&ctl, &mut f, P1));
    ctl.records[i].state = 1;
    assert!(active(&ctl, &mut f, P1));
    assert!(!act3::active(&ctl, &mut f, i, P1, act3::npc::ALKOR));
    setf(&mut f, P1, S, 2);
    assert!(!active(&ctl, &mut f, P1));
    // An untold part.
    give(&mut f, P1, &[b"qhr "]);
    assert!(active(&ctl, &mut f, P1));
    setf(&mut f, P1, S, 6);
    assert!(!active(&ctl, &mut f, P1));
    // The Will untold: only with the orb intact.
    give(&mut f, P1, &[b"qf2 "]);
    assert!(active(&ctl, &mut f, P1));
    smash(&mut ctl);
    assert!(!active(&ctl, &mut f, P1));
    // All four parts with 18.5 clear (the other bits set).
    f.p(P1).items = vec![*b"qhr ", *b"qey ", *b"qbr ", *b"qf1 "];
    for b in [3, 4, 7] {
        setf(&mut f, P1, S, b);
    }
    assert!(active(&ctl, &mut f, P1));
    setf(&mut f, P1, S, 5);
    assert!(!active(&ctl, &mut f, P1));
}

// Covers: specs/world/quests-act3.md §4.4
#[test]
fn cain_messages_and_chat_end() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.add_player(P2, 75);
    let i = idx(&ctl);
    let msg = |p: UnitId, m: u32| {
        ev(
            event::SCROLL_MESSAGE,
            p,
            Some(CAIN3_U),
            u32::from(act3::npc::CAIN3),
            m,
        )
    };
    assert!(!ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    call(&mut ctl, &mut f, C, msg(P1, 543));
    assert_eq!(ctl.records[i].state, 2);
    assert!(f.flags(P1).get(S, 2));
    assert!(x(&ctl).cain_started);
    assert!(ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    assert!(f.f.sent.is_empty());
    for (m, b) in [(544, 6), (545, 3), (546, 4), (547, 5), (548, 7)] {
        call(&mut ctl, &mut f, C, msg(P2, m));
        assert!(f.flags(P2).get(S, b), "{m}");
    }
    // Another NPC's message: nothing.
    let mut m = msg(P1, 544);
    m.a = u32::from(act3::npc::ALKOR);
    call(&mut ctl, &mut f, C, m);
    assert!(!f.flags(P1).get(S, 6));
    // Chat end at another NPC: nothing.
    let end = |n: UnitId| ev(event::NPC_DEACTIVATE, P1, Some(n), 0, 0);
    call(&mut ctl, &mut f, C, end(ALKOR_U));
    assert!(ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    // At Cain: status 1 to all (both lack 18.0 / 18.15), callback cleared,
    // +0x01 stays.
    setf(&mut f, P2, S, 15);
    ctl.records[i].flags = 4;
    call(&mut ctl, &mut f, C, end(CAIN3_U));
    let r = &ctl.records[i];
    assert_eq!((r.status, r.flags), (1, 0));
    assert!(!r.has_callback(event::NPC_DEACTIVATE));
    assert!(x(&ctl).cain_started);
    // Chain 16's F: P2 has 18.15 → not sent.
    let s = sent_5d(&f);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].0, P1);
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §4.5
#[test]
fn level_change_and_leave() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    let lvl = |old: u32, new: u32| ev(event::CHANGED_LEVEL, P1, Some(P1), old, new);
    call(&mut ctl, &mut f, C, lvl(76, 77));
    assert_eq!(ctl.records[i].state, 1);
    ctl.records[i].state = 2;
    call(&mut ctl, &mut f, C, lvl(76, 77));
    assert_eq!(ctl.records[i].state, 1);
    ctl.records[i].state = 3;
    call(&mut ctl, &mut f, C, lvl(76, 77));
    assert_eq!(ctl.records[i].state, 3);
    ctl.records[i].state = 0;
    call(&mut ctl, &mut f, C, lvl(76, 78));
    assert_eq!(ctl.records[i].state, 0);
    ctl.records[i].not_intro = false;
    call(&mut ctl, &mut f, C, lvl(76, 77));
    assert_eq!(ctl.records[i].state, 0);
    // Event 10: a bare `ret` (handled, nothing changes).
    ctl.records[i].guids.add(1);
    let before = ctl.clone();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_LEAVES_GAME, P1, Some(P1), 0, 0),
    );
    assert_eq!(ctl, before);
    assert!(f.f.sent.is_empty());
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §4.5
#[test]
fn pick_up_vectors() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    setf(&mut f, P1, 15, 0);
    ctl.records[i].flags = 0x21;
    ctl.records[i].status = 3;
    // Test vector: `qhr ` with `qey `, `qbr `, no `qf1 ` → v 3.
    give(&mut f, P1, &[b"qey ", b"qbr ", b"qhr "]);
    pick(&mut ctl, &mut f, P1, b"qhr ");
    assert_eq!(f.f.sent, vec![(P1, vec![0x5D, 0x10, 0x21, 3, 0, 0])]);
    // The status byte is not written.
    assert_eq!(ctl.records[i].status, 3);
    // Test vector: `qf1 ` holding all parts, 18.5 clear → v 5; set → 7.
    f.f.sent.clear();
    give(&mut f, P1, &[b"qf1 "]);
    pick(&mut ctl, &mut f, P1, b"qf1 ");
    assert_eq!(f.f.sent, vec![(P1, vec![0x5D, 0x10, 0x21, 5, 0, 0])]);
    setf(&mut f, P1, S, 5);
    f.f.sent.clear();
    pick(&mut ctl, &mut f, P1, b"qf1 ");
    assert_eq!(f.f.sent[0].1[3], 7);
    // All held: the other parts → 7.
    for c in [b"qey ", b"qhr ", b"qbr "] {
        f.f.sent.clear();
        pick(&mut ctl, &mut f, P1, c);
        assert_eq!(f.f.sent[0].1[3], 7);
    }
    // To the picker only.
    f.add_player(P2, 75);
    f.f.sent.clear();
    pick(&mut ctl, &mut f, P1, b"qey ");
    assert_eq!(f.f.sent.len(), 1);
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §4.5
#[test]
fn pick_up_first_missing() {
    let v = |held: &[&[u8; 4]], picked: &[u8; 4]| {
        let (mut ctl, _) = control();
        let mut f = Fake3::new();
        setf(&mut f, P1, 15, 0);
        give(&mut f, P1, held);
        pick(&mut ctl, &mut f, P1, picked);
        f.f.sent[0].1[3]
    };
    // `qf1 `: eye 1, brain 2, heart 4.
    assert_eq!(v(&[b"qf1 "], b"qf1 "), 1);
    assert_eq!(v(&[b"qey "], b"qf1 "), 2);
    assert_eq!(v(&[b"qey ", b"qbr "], b"qf1 "), 4);
    // `qey `: brain 2, flail 3, heart 4.
    assert_eq!(v(&[], b"qey "), 2);
    assert_eq!(v(&[b"qbr "], b"qey "), 3);
    assert_eq!(v(&[b"qbr ", b"qf1 "], b"qey "), 4);
    // `qhr `: brain 2, flail 3, eye 1.
    assert_eq!(v(&[], b"qhr "), 2);
    assert_eq!(v(&[b"qbr "], b"qhr "), 3);
    assert_eq!(v(&[b"qbr ", b"qf1 "], b"qhr "), 1);
    // `qbr `: heart 4, flail 3, eye 1.
    assert_eq!(v(&[], b"qbr "), 4);
    assert_eq!(v(&[b"qhr "], b"qbr "), 3);
    assert_eq!(v(&[b"qhr ", b"qf1 "], b"qbr "), 1);
}

// Covers: specs/world/quests-act3.md §4.5, §edge-cases-original-bugs r2
#[test]
fn pick_up_gates_and_will() {
    // 15.0 clear or 18.0 set: nothing.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    pick(&mut ctl, &mut f, P1, b"qey ");
    setf(&mut f, P1, 15, 0);
    setf(&mut f, P1, S, 0);
    pick(&mut ctl, &mut f, P1, b"qey ");
    assert!(f.f.sent.is_empty());
    // Edge case 2: the Will sends nothing; status ≠ 0 → 7, status 0
    // stays 0; flags untouched.
    let mut f = Fake3::new();
    setf(&mut f, P1, 15, 0);
    ctl.records[i].status = 1;
    ctl.records[i].flags = 5;
    pick(&mut ctl, &mut f, P1, b"qf2 ");
    assert_eq!((ctl.records[i].status, ctl.records[i].flags), (7, 5));
    ctl.records[i].status = 0;
    pick(&mut ctl, &mut f, P1, b"qf2 ");
    assert_eq!(ctl.records[i].status, 0);
    // Other codes: nothing.
    pick(&mut ctl, &mut f, P1, b"bbb ");
    assert!(f.f.sent.is_empty());
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §4.5
#[test]
fn game_start() {
    let start = |ctl: &mut QuestControl, f: &mut Fake3| {
        call(
            ctl,
            f,
            C,
            ev(event::PLAYER_STARTED_GAME, P1, Some(P1), 0, 0),
        );
    };
    let (mut ctl, _) = control();
    let i = idx(&ctl);
    // Init: status 1, state 0.
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (0, 1));
    // 18.0: nothing (even with 18.7).
    let mut f = Fake3::new();
    setf(&mut f, P1, S, 0);
    setf(&mut f, P1, S, 7);
    start(&mut ctl, &mut f);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (0, 1));
    // 18.7 → status 7, state 2.
    let mut f = Fake3::new();
    setf(&mut f, P1, S, 7);
    start(&mut ctl, &mut f);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (2, 7));
    // 18.2 → state 2, status 7.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    setf(&mut f, P1, S, 2);
    start(&mut ctl, &mut f);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (2, 7));
    // Neither, but a part count ≠ 0 → status 7, state 2.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    ctl.records[i].extra.act3.q2.flails = 1;
    start(&mut ctl, &mut f);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (2, 7));
    // Nothing: unchanged; nothing sent in any case.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    ctl.records[i].extra.act3.q2.wills = 1;
    start(&mut ctl, &mut f);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (0, 1));
    assert!(f.f.sent.is_empty());
    no_unhandled(&f);
}

// Covers: specs/world/quests-act3.md §4.9, §edge-cases-original-bugs r16
#[test]
fn status_function() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    // 15.0 clear → 0 (even holding the Will).
    give(&mut f, P1, &[b"qf2 "]);
    assert_eq!(status_of(&ctl, &mut f, P1), 0);
    setf(&mut f, P1, 15, 0);
    // Test vector: the Will, orb intact → 6; smashed → 12. 18.0 is not
    // tested (edge case 16).
    setf(&mut f, P1, S, 0);
    assert_eq!(status_of(&ctl, &mut f, P1), 6);
    smash(&mut ctl);
    assert_eq!(status_of(&ctl, &mut f, P1), 12);
    // Parts: none → 18.2 ? 1 : (state > 1 ? 1 : 0).
    f.p(P1).items.clear();
    assert_eq!(status_of(&ctl, &mut f, P1), 0);
    ctl.records[i].state = 2;
    assert_eq!(status_of(&ctl, &mut f, P1), 1);
    ctl.records[i].not_intro = false;
    assert_eq!(status_of(&ctl, &mut f, P1), 1);
    ctl.records[i].state = 1;
    assert_eq!(status_of(&ctl, &mut f, P1), 0);
    setf(&mut f, P1, S, 2);
    assert_eq!(status_of(&ctl, &mut f, P1), 1);
    // n > 0: no eye 1, no brain 2, no heart 4, else flail ? 7 : 3.
    give(&mut f, P1, &[b"qf1 "]);
    assert_eq!(status_of(&ctl, &mut f, P1), 1);
    f.p(P1).items = vec![*b"qey "];
    assert_eq!(status_of(&ctl, &mut f, P1), 2);
    give(&mut f, P1, &[b"qbr "]);
    assert_eq!(status_of(&ctl, &mut f, P1), 4);
    give(&mut f, P1, &[b"qhr "]);
    assert_eq!(status_of(&ctl, &mut f, P1), 3);
    // All four: 18.5 clear → 7, set → 5.
    give(&mut f, P1, &[b"qf1 "]);
    assert_eq!(status_of(&ctl, &mut f, P1), 7);
    setf(&mut f, P1, S, 5);
    assert_eq!(status_of(&ctl, &mut f, P1), 5);
    // Chain 19 absent: the orb counts as intact.
    give(&mut f, P1, &[b"qf2 "]);
    let j = ctl.find(19).unwrap();
    ctl.records.remove(j);
    let i = idx(&ctl);
    let pf = f.flags(P1);
    assert_eq!(act3::status(&ctl, &mut f, i, P1, &pf), Some(6));
}

// Covers: specs/world/quests-act3.md §4.6, §4.2, §edge-cases-original-bugs r3
#[test]
fn chest_vector() {
    // Test vector: quest seed {12345, 666} → lo' 22752887 → 7 gold piles,
    // before the part; then the treasure.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.add_player(P2, 80);
    f.add_player(P3, 1);
    f.p(P3).act = Some(0);
    // P2 holds the heart, P3 (elsewhere) holds neither: 2 drops (P1, P3).
    give(&mut f, P2, &[b"qhr "]);
    ctl.seed = Seed::new(12345, 666);
    act3::chest_operate(&mut ctl, &mut f, CHEST_U, P1, act3::KhalimChest::Heart);
    assert_eq!(ctl.seed.lo, 22_752_887);
    let mut want = vec![format!("gold {}", CHEST_U.0); 7];
    want.push(format!("qdrop {} qhr  2 true", CHEST_U.0));
    want.push(format!("qdrop {} qhr  2 true", CHEST_U.0));
    want.push(format!("treasure {} 4", CHEST_U.0));
    assert_eq!(f.log(), want);
    let q = x(&ctl);
    assert_eq!((q.drop_count, q.hearts), (2, 2));
    assert!(q.heart_dropped && !q.eye_dropped && !q.brain_dropped);
    assert!(f.f.sent.is_empty());
}

// Covers: specs/world/quests-act3.md §4.6
#[test]
fn chest_counts_and_gate() {
    // The brain chest; P1 holds the Will (no drop), P2 counts; the one
    // drop fails: count and flag unchanged.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.add_player(P2, 80);
    give(&mut f, P1, &[b"qf2 "]);
    f.drops = vec![false];
    let lo = Seed::new(ctl.seed.lo, ctl.seed.hi).step();
    act3::chest_operate(&mut ctl, &mut f, CHEST_U, P2, act3::KhalimChest::Brain);
    let gold = f.log().iter().filter(|l| l.starts_with("gold")).count();
    assert_eq!(gold as u32, lo % 5 + 5);
    let q = x(&ctl);
    assert_eq!((q.drop_count, q.brains), (1, 0));
    assert!(!q.brain_dropped);
    assert!(f.log().last().unwrap().starts_with("treasure"));
    // The eye chest drops `qey `.
    let mut f = Fake3::new();
    act3::chest_operate(&mut ctl, &mut f, CHEST_U, P1, act3::KhalimChest::Eye);
    assert!(f
        .log()
        .contains(&format!("qdrop {} qey  2 true", CHEST_U.0)));
    assert_eq!(x(&ctl).eyes, 1);
    assert!(x(&ctl).eye_dropped);
    // The gate refuses: nothing, no seed step.
    let mut f = Fake3::new();
    f.chest_gate = false;
    let seed = ctl.seed;
    act3::chest_operate(&mut ctl, &mut f, CHEST_U, P1, act3::KhalimChest::Eye);
    assert!(f.log().is_empty());
    assert_eq!(ctl.seed, seed);
    // No chain 16: gold and treasure only.
    let j = idx(&ctl);
    ctl.records.remove(j);
    let mut f = Fake3::new();
    act3::chest_operate(&mut ctl, &mut f, CHEST_U, P1, act3::KhalimChest::Eye);
    assert!(!f.log().iter().any(|l| l.starts_with("qdrop")));
    assert!(f.log().last().unwrap().starts_with("treasure"));
}

// Covers: specs/world/quests-act3.md §4.7
#[test]
fn sewer_stairs() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    f.f.objects.insert(STAIRS_U, (0x88, 366, 0));
    act3::stairs_init(&mut ctl, &mut f, STAIRS_U);
    assert!(x(&ctl).stairs_known);
    assert_eq!(x(&ctl).stairs_guid, 0x88);
    assert_eq!(f.f.objects[&STAIRS_U].2, 0);
    // Closed: no warp.
    act3::stairs_operate(&mut ctl, &mut f, STAIRS_U, P1);
    assert!(!f.log().iter().any(|l| l.starts_with("stairs warp")));
    ctl.records[i].extra.act3.q2.stairs_mode = 2;
    act3::stairs_init(&mut ctl, &mut f, STAIRS_U);
    assert_eq!(f.f.objects[&STAIRS_U].2, 2);
    act3::stairs_operate(&mut ctl, &mut f, STAIRS_U, P1);
    assert!(f
        .log()
        .contains(&format!("stairs warp {} {}", STAIRS_U.0, P1.0)));
    // Intro: mode 2 whatever +0x08 says.
    ctl.records[i].extra.act3.q2.stairs_mode = 0;
    ctl.records[i].not_intro = false;
    f.f.objects.insert(STAIRS_U, (0x88, 366, 0));
    act3::stairs_init(&mut ctl, &mut f, STAIRS_U);
    assert_eq!(f.f.objects[&STAIRS_U].2, 2);
}

// Covers: specs/world/quests-act3.md §4.7
#[test]
fn sewer_lever() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    f.f.frame = 100;
    f.f.objects.insert(LEVER_U, (0x89, 367, 0));
    // Lever init: not intro → nothing.
    act3::lever_init(&mut ctl, &mut f, LEVER_U);
    assert!(f.log().is_empty());
    // Stairs unknown: the lever does nothing.
    act3::lever_operate(&mut ctl, &mut f, LEVER_U, P1);
    assert!(f.log().is_empty());
    // Stairs known but gone: nothing.
    ctl.records[i].extra.act3.q2.stairs_known = true;
    ctl.records[i].extra.act3.q2.stairs_guid = 0x88;
    act3::lever_operate(&mut ctl, &mut f, LEVER_U, P1);
    assert!(f.log().is_empty());
    // Stairs present: mode 1 with the end-animation event, +0x08 := 2,
    // event 7 at frame + 30, FX 9.
    f.f.objects.insert(STAIRS_U, (0x88, 366, 0));
    act3::lever_operate(&mut ctl, &mut f, LEVER_U, P1);
    assert_eq!(
        f.log(),
        vec![
            format!("mode {} 1", LEVER_U.0),
            format!("event1 {} {}", LEVER_U.0, 100 + 0x10),
            format!("event7 {} 130", LEVER_U.0),
        ]
    );
    assert_eq!(x(&ctl).stairs_mode, 2);
    assert_eq!(ctl.fx, 9);
    assert!(f.f.sent.contains(&(P1, vec![0x89, 9])));
    // Lever not in mode 0: nothing.
    let n = f.log().len();
    act3::lever_operate(&mut ctl, &mut f, LEVER_U, P1);
    assert_eq!(f.log().len(), n);
    // Lever event 7: the stairs take +0x08 (2), no further event.
    act3::lever_event(&mut ctl, &mut f, LEVER_U);
    assert_eq!(f.f.objects[&STAIRS_U].2, 2);
    assert_eq!(f.log().len(), n + 1);
    // Intro lever init: mode 2.
    ctl.records[i].not_intro = false;
    act3::lever_init(&mut ctl, &mut f, LEVER_U);
    assert_eq!(f.f.objects[&LEVER_U].2, 2);
}

// Covers: specs/world/quests-act3.md §4.7
#[test]
fn lever_event_cases() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    // Not known: nothing.
    ctl.records[i].extra.act3.q2.stairs_mode = 1;
    act3::lever_event(&mut ctl, &mut f, LEVER_U);
    assert_eq!(x(&ctl).stairs_mode, 1);
    // Known, stairs gone → +0x08 := 2.
    ctl.records[i].extra.act3.q2.stairs_known = true;
    ctl.records[i].extra.act3.q2.stairs_guid = 0x88;
    act3::lever_event(&mut ctl, &mut f, LEVER_U);
    assert_eq!(x(&ctl).stairs_mode, 2);
    assert!(f.log().is_empty());
    // Present with +0x08 ≠ 2: mode := +0x08, then +0x08 := 2 and an
    // end-animation event on the stairs.
    f.f.objects.insert(STAIRS_U, (0x88, 366, 0));
    ctl.records[i].extra.act3.q2.stairs_mode = 1;
    act3::lever_event(&mut ctl, &mut f, LEVER_U);
    assert_eq!(
        f.log(),
        vec![
            format!("mode {} 1", STAIRS_U.0),
            format!("event1 {} {}", STAIRS_U.0, 0x10),
        ]
    );
    assert_eq!(x(&ctl).stairs_mode, 2);
}

// Covers: specs/world/quests-act3.md §4.8
#[test]
fn will_cubed_counts() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let i = idx(&ctl);
    {
        let q = &mut ctl.records[i].extra.act3.q2;
        (q.eyes, q.hearts, q.brains, q.flails) = (1, 2, 1, 1);
    }
    setf(&mut f, P1, S, 0);
    let flags = f.flags(P1);
    act3::will_cubed(&mut ctl, &mut f, P1);
    let q = x(&ctl);
    assert_eq!(
        (q.eyes, q.hearts, q.brains, q.flails, q.wills),
        (0, 1, 0, 0, 1)
    );
    // No bit changes, nothing sent; the record's state and status stay.
    assert_eq!(f.flags(P1), flags);
    assert!(f.f.sent.is_empty());
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (0, 1));
    // No chain 16: nothing.
    ctl.records.remove(i);
    act3::will_cubed(&mut ctl, &mut f, P1);
}
