// Spec: specs/world/quests-act3.md §6 (A3Q4 The Golden Bird)
//! Chain 18 callback by callback: the boss choice and its kill, chat,
//! messages, chat end, the Potion of Life, pick-up, join / leave, game
//! start, the status function and Alkor's map-AI hooks.

use super::*;

const C: u8 = 18;
const S: u8 = 20;
const J34: [u8; 4] = *b"j34 ";
const G34: [u8; 4] = *b"g34 ";
const P3: UnitId = UnitId(3);
const MON: UnitId = UnitId(0x50);
const ITEM: UnitId = UnitId(0x51);

fn bits(f: &mut Fake3, p: UnitId, slot: u8, bs: &[u8]) {
    let d = usize::from(f.f.difficulty);
    for &b in bs {
        f.p(p).quests.flags[d].set(slot, b);
    }
}

fn has(f: &Fake3, p: UnitId, b: u8) -> bool {
    f.flags(p).get(S, b)
}

fn idx(ctl: &QuestControl) -> usize {
    ctl.find(C).unwrap()
}

fn x(ctl: &QuestControl) -> &act3::q4::Extra {
    &ctl.record(C).unwrap().extra.act3.q4
}

fn xm(ctl: &mut QuestControl) -> &mut act3::q4::Extra {
    &mut ctl.record_mut(C).unwrap().extra.act3.q4
}

fn ev(event: u8, target: Option<UnitId>, player: UnitId) -> EventArgs {
    EventArgs {
        event,
        target,
        player: Some(player),
        ..EventArgs::default()
    }
}

fn msg(ctl: &mut QuestControl, f: &mut Fake3, p: UnitId, n: UnitId, m: u16) {
    let class = f.monster_class(n).unwrap();
    let args = EventArgs {
        event: event::SCROLL_MESSAGE,
        target: Some(n),
        player: Some(p),
        a: u32::from(class),
        b: u32::from(m),
    };
    call(ctl, f, C, args);
}

fn status_of(ctl: &QuestControl, f: &mut Fake3, p: UnitId) -> u8 {
    let r = f.flags(p);
    act3::status(ctl, f, idx(ctl), p, &r).unwrap()
}

fn setup() -> (QuestControl, Fake3) {
    let mut f = Fake3::new();
    f.f.monsters.insert(
        MON,
        (
            0x50,
            100,
            UnitKind::Monster {
                class: 100,
                superunique: None,
                owner: None,
            },
        ),
    );
    f.f.chains.insert(MON, QuestChain::default());
    f.acts.insert(MON, 2);
    (control().0, f)
}

fn to_5d(f: &Fake3) -> Vec<UnitId> {
    sent_5d(f).iter().map(|m| m.0).collect()
}

// ------------------------------------------------------------ §6.2

// Covers: specs/world/quests-act3.md §6.2
#[test]
fn boss_choice() {
    let (mut ctl, mut f) = setup();
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, Some(0xBF));
    let i = idx(&ctl);
    assert_eq!(f.f.chains[&MON].0, vec![C]);
    assert!(ctl.records[i].has_callback(event::MONSTER_KILLED));
    let e = x(&ctl);
    assert!(!e.may_choose && e.chosen);
    assert_eq!(e.boss_guid, 0x50);
    // Only one boss at a time.
    let other = UnitId(0x52);
    f.f.chains.insert(other, QuestChain::default());
    f.acts.insert(other, 2);
    act3::choose_bird_boss(&mut ctl, &mut f, other, 100, Some(0));
    assert!(f.f.chains[&other].0.is_empty());
    // Removal of another monster: nothing; of the boss: chosen again.
    act3::bird_boss_removed(&mut ctl, &mut f, other);
    assert!(x(&ctl).chosen);
    act3::bird_boss_removed(&mut ctl, &mut f, MON);
    assert!(!x(&ctl).chosen && x(&ctl).may_choose);
    act3::choose_bird_boss(&mut ctl, &mut f, other, 100, Some(0));
    assert_eq!(f.f.chains[&other].0, vec![C]);
    assert_eq!(x(&ctl).boss_guid, other.0);
    // Removal when no boss is chosen: nothing.
    let (mut ctl, mut f) = setup();
    act3::bird_boss_removed(&mut ctl, &mut f, MON);
    assert!(!x(&ctl).chosen && x(&ctl).may_choose);
}

// Covers: specs/world/quests-act3.md §6.2
#[test]
fn boss_choice_refusals() {
    type Tweak = fn(&mut QuestControl, &mut Fake3) -> (u16, u8);
    let cases: [Tweak; 6] = [
        |ctl, _| {
            ctl.record_mut(C).unwrap().not_intro = false;
            (100, 0)
        },
        |_, _| (407, 0),
        |_, _| (100, 0x40),
        |_, f| {
            f.acts.insert(MON, 1);
            (100, 0)
        },
        |ctl, _| {
            xm(ctl).to_drop = false;
            (100, 0)
        },
        |ctl, _| {
            // A Gidbinn boss is being spawned (chain 17's +0x04).
            ctl.record_mut(17).unwrap().extra.act3.q3.boss_spawning = true;
            (100, 0)
        },
    ];
    for (n, tweak) in cases.iter().enumerate() {
        let (mut ctl, mut f) = setup();
        let (class, flags) = tweak(&mut ctl, &mut f);
        act3::choose_bird_boss(&mut ctl, &mut f, MON, class, Some(flags));
        assert!(f.f.chains[&MON].0.is_empty(), "case {n}");
        assert!(!x(&ctl).chosen, "case {n}");
        assert!(!ctl.record(C).unwrap().has_callback(event::MONSTER_KILLED));
    }
    // Chain 17 absent → allowed.
    let (mut ctl, mut f) = setup();
    let j = ctl.find(17).unwrap();
    ctl.records.remove(j);
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, Some(0));
    assert!(x(&ctl).chosen);
}

// ------------------------------------------------------------ §6.3

// Covers: specs/world/quests-act3.md §6.3
#[test]
fn boss_kill_drops_the_figurine() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, Some(0));
    // Drop fails → a boss may be chosen again.
    f.drops = vec![false];
    ctl.monster_killed(&mut f, MON, Some(P1));
    assert_eq!(f.log(), vec!["qdrop 80 j34  2 false"]);
    assert!(!x(&ctl).chosen && x(&ctl).may_choose && x(&ctl).to_drop);
    assert!(ctl.record(C).unwrap().has_callback(event::MONSTER_KILLED));
    // Needs +0x02.
    ctl.monster_killed(&mut f, MON, Some(P1));
    assert_eq!(f.log().len(), 1);
    // Chosen again and killed: created.
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, Some(0));
    ctl.monster_killed(&mut f, MON, Some(P1));
    let r = ctl.record(C).unwrap();
    assert!(!r.has_callback(event::MONSTER_KILLED));
    assert_eq!((r.state, r.status), (1, 1));
    let e = x(&ctl);
    assert!(!e.chosen && !e.to_drop && e.dropped && !e.may_choose);
    assert_eq!(e.held, 1);
    assert_eq!(to_5d(&f), vec![P1, P2]);
}

// Covers: specs/world/quests-act3.md §6.3
#[test]
fn boss_kill_guards() {
    // A killer with 20.0 → nothing.
    let (mut ctl, mut f) = setup();
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, Some(0));
    bits(&mut f, P1, S, &[0]);
    ctl.monster_killed(&mut f, MON, Some(P1));
    assert!(f.log().is_empty() && x(&ctl).chosen);
    // No killing player → the drop.
    ctl.monster_killed(&mut f, MON, None);
    assert_eq!(f.log().len(), 1);
    // State ≠ 0 and status ≠ 0 stay.
    let (mut ctl, mut f) = setup();
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, Some(0));
    let i = idx(&ctl);
    (ctl.records[i].state, ctl.records[i].status) = (2, 2);
    ctl.monster_killed(&mut f, MON, Some(P1));
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (2, 2));
    assert!(sent_5d(&f).is_empty());
    // +0x0C clear or intro → nothing.
    let (mut ctl, mut f) = setup();
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, Some(0));
    xm(&mut ctl).to_drop = false;
    ctl.monster_killed(&mut f, MON, Some(P1));
    assert!(f.log().is_empty() && x(&ctl).chosen);
    xm(&mut ctl).to_drop = true;
    ctl.record_mut(C).unwrap().not_intro = false;
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::MONSTER_KILLED, Some(MON), P1),
    );
    assert!(f.log().is_empty());
}

// ------------------------------------------------------------ §6.4

// Covers: specs/world/quests-act3.md §6.4
#[test]
fn chat_lists() {
    let (mut ctl, mut f) = setup();
    // Nothing held, nothing set → nothing.
    assert!(text(&mut ctl, &mut f, C, P1, CAIN3_U).is_empty());
    // Figurine: Cain / Asheara → 0 (20.2 clear) or 7; others → 1.
    f.p(P1).items.push(J34);
    assert_eq!(text(&mut ctl, &mut f, C, P1, CAIN3_U), vec![(527, 0)]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, ASHEARA_U), vec![(528, 2)]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, MESHIF2_U), vec![(529, 0)]);
    bits(&mut f, P1, S, &[2]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, CAIN3_U), vec![(527, 2)]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, ASHEARA_U), vec![(528, 2)]);
    // Bird: Cain with 20.4 clear → 3; Alkor and others → 2.
    f.p(P1).items = vec![G34];
    assert_eq!(text(&mut ctl, &mut f, C, P1, CAIN3_U), vec![(531, 0)]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, ALKOR_U), vec![(534, 0)]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, HRATLI_U), vec![(532, 2)]);
    bits(&mut f, P1, S, &[4]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, CAIN3_U), vec![(531, 2)]);
    // 20.1 → 5.
    bits(&mut f, P1, S, &[1]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, ALKOR_U), vec![(538, 0)]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, CAIN3_U), vec![(540, 2)]);
    // Alkor with +0x00 → nothing.
    xm(&mut ctl).bird_brought = true;
    assert!(text(&mut ctl, &mut f, C, P1, ALKOR_U).is_empty());
    assert_eq!(text(&mut ctl, &mut f, C, P1, CAIN3_U), vec![(540, 2)]);
}

// Covers: specs/world/quests-act3.md §6.4
#[test]
fn chat_when_done_needs_the_guid() {
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[0]);
    f.p(P1).items.push(G34);
    assert!(text(&mut ctl, &mut f, C, P1, ALKOR_U).is_empty());
    let i = idx(&ctl);
    ctl.records[i].guids.add(1);
    assert_eq!(text(&mut ctl, &mut f, C, P1, NATALYA_U), vec![(542, 2)]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, ALKOR_U), vec![(538, 2)]);
}

// Covers: specs/world/quests-act3.md §6.4, §edge-cases-original-bugs r8
#[test]
fn table_state_4_never_selected() {
    let state4: [(u16, u32); 4] = [(534, 2), (535, 2), (536, 2), (537, 2)];
    let npcs = [
        CAIN3_U, ASHEARA_U, HRATLI_U, ALKOR_U, ORMUS_U, MESHIF2_U, NATALYA_U,
    ];
    for mask in 0u32..64 {
        let (mut ctl, mut f) = setup();
        let bs: Vec<u8> = [0u8, 1, 2, 4]
            .iter()
            .enumerate()
            .filter(|(k, _)| mask & (1 << k) != 0)
            .map(|(_, &b)| b)
            .collect();
        bits(&mut f, P1, S, &bs);
        if mask & 16 != 0 {
            f.p(P1).items.push(G34);
        }
        if mask & 32 != 0 {
            f.p(P1).items.push(J34);
        }
        let i = idx(&ctl);
        ctl.records[i].guids.add(1);
        for state in 0..8 {
            ctl.records[i].state = state;
            for n in npcs {
                let l = text(&mut ctl, &mut f, C, P1, n);
                assert!(!l.iter().any(|e| state4.contains(e)), "{mask} {n:?}");
            }
        }
    }
}

// Covers: specs/world/quests-act3.md §6.4
#[test]
fn wants_to_talk() {
    let (ctl, mut f) = setup();
    let i = idx(&ctl);
    let act = |f: &mut Fake3, n: u16| act3::active(&ctl, f, i, P1, n);
    assert!(!act(&mut f, act3::npc::ALKOR));
    f.p(P1).items = vec![G34];
    assert!(act(&mut f, act3::npc::ALKOR));
    assert!(act(&mut f, act3::npc::CAIN3));
    assert!(!act(&mut f, act3::npc::MESHIF2));
    bits(&mut f, P1, S, &[4]);
    assert!(!act(&mut f, act3::npc::CAIN3));
    f.p(P1).items = vec![J34];
    assert!(act(&mut f, act3::npc::MESHIF2));
    assert!(act(&mut f, act3::npc::CAIN3));
    assert!(!act(&mut f, act3::npc::ALKOR));
    assert!(!act(&mut f, act3::npc::ASHEARA));
    bits(&mut f, P1, S, &[2]);
    assert!(!act(&mut f, act3::npc::CAIN3));
    // 20.1 or 20.0 → never.
    f.p(P1).items = vec![J34, G34];
    assert!(act(&mut f, act3::npc::ALKOR));
    bits(&mut f, P1, S, &[1]);
    assert!(!act(&mut f, act3::npc::ALKOR));
    assert!(!act(&mut f, act3::npc::MESHIF2));
    let (ctl, mut f) = setup();
    f.p(P1).items = vec![G34];
    bits(&mut f, P1, S, &[0]);
    assert!(!act3::active(&ctl, &mut f, i, P1, act3::npc::ALKOR));
}

// ------------------------------------------------------------ §6.5

// Covers: specs/world/quests-act3.md §6.5
#[test]
fn cain_527_and_chat_end() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    f.add_player(P3, 75);
    bits(&mut f, P3, S, &[1]);
    f.f.party.insert(P1, vec![P1, P2, P3]);
    f.p(P1).items.push(J34);
    ctl.record_mut(C).unwrap().state = 1;
    msg(&mut ctl, &mut f, P1, CAIN3_U, 527);
    let i = idx(&ctl);
    assert!(has(&f, P1, 2));
    assert_eq!(ctl.records[i].state, 2);
    assert!(x(&ctl).pend_cain1);
    assert!(ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    assert_eq!(x(&ctl).party_bit, 2);
    // Party: P2 gets 20.2, P3 (20.1) does not.
    assert!(has(&f, P2, 2) && !has(&f, P3, 2));
    // Refresh: Cain's text re-sent with chain 18's new line.
    assert!(f
        .log()
        .iter()
        .any(|l| l.starts_with("0x27 32") && l.contains("(527, 2)")));
    // Chat end with Cain: status 2 to all.
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(CAIN3_U), P1),
    );
    assert_eq!(ctl.records[i].status, 2);
    assert!(!x(&ctl).pend_cain1);
    assert_eq!(to_5d(&f), vec![P1, P2, P3]);
    // Edge case 6: never cleared; again → nothing.
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(CAIN3_U), P1),
    );
    assert!(sent_5d(&f).is_empty());
    assert!(ctl.records[i].has_callback(event::NPC_DEACTIVATE));
}

// Covers: specs/world/quests-act3.md §6.5
#[test]
fn cain_527_outside_state_1() {
    let (mut ctl, mut f) = setup();
    ctl.record_mut(C).unwrap().state = 3;
    msg(&mut ctl, &mut f, P1, CAIN3_U, 527);
    assert!(has(&f, P1, 2));
    assert_eq!(ctl.record(C).unwrap().state, 3);
    assert!(!x(&ctl).pend_cain1);
    assert!(!ctl.record(C).unwrap().has_callback(event::NPC_DEACTIVATE));
    // 20.0 → nothing.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[0]);
    msg(&mut ctl, &mut f, P1, CAIN3_U, 527);
    assert!(!has(&f, P1, 2));
}

// Covers: specs/world/quests-act3.md §6.5, §edge-cases-original-bugs r7
#[test]
fn cain_531_relies_on_an_earlier_install() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    f.f.party.insert(P1, vec![P1, P2]);
    f.p(P1).items.push(G34);
    msg(&mut ctl, &mut f, P1, CAIN3_U, 531);
    let i = idx(&ctl);
    assert!(has(&f, P1, 4) && has(&f, P2, 4));
    assert!(x(&ctl).pend_cain2);
    assert_eq!(x(&ctl).party_bit, 4);
    // No callback 2: a chat end through the dispatch does nothing.
    assert!(!ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    f.f.sent.clear();
    ctl.npc_deactivate(&mut f, P1, CAIN3_U);
    assert!(x(&ctl).pend_cain2);
    assert_eq!(ctl.records[i].status, 0);
    // With an earlier install the chat end sends status 4.
    ctl.records[i].callbacks |= 1 << event::NPC_DEACTIVATE;
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(CAIN3_U), P1),
    );
    assert!(!x(&ctl).pend_cain2);
    assert_eq!(ctl.records[i].status, 4);
    assert_eq!(to_5d(&f), vec![P1, P2]);
    // Both pending: status 2, then 4.
    xm(&mut ctl).pend_cain1 = true;
    xm(&mut ctl).pend_cain2 = true;
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(CAIN3_U), P1),
    );
    assert_eq!(ctl.records[i].status, 4);
    assert_eq!(to_5d(&f), vec![P1, P2, P1, P2]);
}

// Covers: specs/world/quests-act3.md §6.5
#[test]
fn meshif_trades_the_figurine() {
    let (mut ctl, mut f) = setup();
    xm(&mut ctl).held = 1;
    // Without the figurine → nothing.
    msg(&mut ctl, &mut f, P1, MESHIF2_U, 529);
    assert!(f.log().is_empty());
    f.p(P1).items.push(J34);
    msg(&mut ctl, &mut f, P1, MESHIF2_U, 529);
    assert_eq!(f.log(), vec!["delete j34 ", "reward g34  0 2"]);
    assert_eq!(f.p(P1).items, vec![G34]);
    let r = ctl.record(C).unwrap();
    assert!(r.has_callback(event::NPC_DEACTIVATE));
    assert_eq!(r.state, 3);
    assert!(x(&ctl).pend_meshif);
    assert_eq!(x(&ctl).held, 1);
    // Chat end with Meshif: status 3 to all.
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(MESHIF2_U), P1),
    );
    assert_eq!(ctl.record(C).unwrap().status, 3);
    assert!(!x(&ctl).pend_meshif);
    assert_eq!(to_5d(&f), vec![P1]);
}

// Covers: specs/world/quests-act3.md §6.5
#[test]
fn alkor_takes_the_bird_then_rewards() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    f.f.party.insert(P1, vec![P1, P2]);
    f.p(P1).items.push(G34);
    xm(&mut ctl).held = 1;
    ctl.record_mut(C).unwrap().state = 3;
    msg(&mut ctl, &mut f, P1, ALKOR_U, 534);
    let i = idx(&ctl);
    assert!(f.p(P1).items.is_empty());
    assert_eq!(x(&ctl).held, 0);
    assert_eq!(ctl.records[i].state, 4);
    assert!(has(&f, P1, 1) && has(&f, P2, 1));
    assert!(x(&ctl).pend_alkor);
    assert_eq!(x(&ctl).party_bit, 1);
    assert!(ctl.records[i].has_callback(event::NPC_DEACTIVATE));
    // Chat end with Alkor: status 5 to all, +0x00 := 1.
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(ALKOR_U), P1),
    );
    assert_eq!(ctl.records[i].status, 5);
    assert!(!x(&ctl).pend_alkor && x(&ctl).bird_brought);
    assert!(act3::alkor_bird_brought(&ctl));
    act3::alkor_bird_clear(&mut ctl);
    assert!(!act3::alkor_bird_brought(&ctl));
    // 538: the reward.
    bits(&mut f, P1, S, &[2, 3, 4, 6, 11]);
    f.f.sent.clear();
    f.f.log.clear();
    msg(&mut ctl, &mut f, P1, ALKOR_U, 538);
    let w = f.flags(P1).word(S);
    assert_eq!(w, 1 | 1 << 5 | 1 << 13);
    assert_eq!(ctl.records[i].state, 5);
    assert_eq!(f.log(), vec!["reward xyz  0 2"]);
    assert_eq!(f.f.sent.len(), 1);
    assert_eq!(f.f.sent[0].1[0], 0x28);
    assert!(ctl.records[i].guids.contains(1));
    assert!(ctl.game.get(S, 13));
    // The own seq fn: chains 16 and 17 start.
    assert_eq!(ctl.record(16).unwrap().state, 1);
    assert_eq!(ctl.record(17).unwrap().state, 1);
    // 20.0 now: nothing more.
    msg(&mut ctl, &mut f, P1, ALKOR_U, 538);
    assert_eq!(f.log().len(), 1);
}

// Covers: specs/world/quests-act3.md §6.5
#[test]
fn alkor_refusals() {
    // 534 without the bird, or with 20.1: nothing.
    let (mut ctl, mut f) = setup();
    msg(&mut ctl, &mut f, P1, ALKOR_U, 534);
    assert!(!has(&f, P1, 1) && f.log().is_empty());
    f.p(P1).items.push(G34);
    bits(&mut f, P1, S, &[1]);
    ctl.record_mut(C).unwrap().state = 3;
    msg(&mut ctl, &mut f, P1, ALKOR_U, 534);
    assert!(f.log().is_empty());
    assert_eq!(ctl.record(C).unwrap().state, 3);
    // 534 in an intro record: no state change.
    let (mut ctl, mut f) = setup();
    f.p(P1).items.push(G34);
    ctl.record_mut(C).unwrap().not_intro = false;
    msg(&mut ctl, &mut f, P1, ALKOR_U, 534);
    assert!(has(&f, P1, 1));
    assert_eq!(ctl.record(C).unwrap().state, 0);
    // 538 without 20.1: nothing given; 20.13 from an earlier
    // reward is still read: game 20.13 and the seq fn.
    let (mut ctl, mut f) = setup();
    msg(&mut ctl, &mut f, P1, ALKOR_U, 538);
    assert!(f.log().is_empty() && !ctl.game.get(S, 13));
    bits(&mut f, P1, S, &[13]);
    msg(&mut ctl, &mut f, P1, ALKOR_U, 538);
    assert!(f.log().is_empty() && ctl.game.get(S, 13));
    assert_eq!(ctl.record(17).unwrap().state, 0); // chain 18 not done
}

// ------------------------------------------------------------ §6.6

// Covers: specs/world/quests-act3.md §6.6
#[test]
fn potion_of_life() {
    let (mut ctl, mut f) = setup();
    assert!(!act3::potion_of_life(&mut ctl, &mut f, P1));
    assert!(f.f.sent.is_empty());
    bits(&mut f, P1, S, &[5, 0]);
    f.p(P1).stats.insert(7, 0x3200);
    assert!(act3::potion_of_life(&mut ctl, &mut f, P1));
    assert_eq!(f.p(P1).stats[&7], 0x3200 + 0x1400);
    assert_eq!(f.f.sent, vec![(P1, vec![0x5D, 0x12, 2, 0, 0, 0])]);
    assert!(!has(&f, P1, 5) && has(&f, P1, 0));
    // Once only.
    assert!(!act3::potion_of_life(&mut ctl, &mut f, P1));
    assert_eq!(f.p(P1).stats[&7], 0x4600);
}

// ------------------------------------------------------------ §6.7

// Covers: specs/world/quests-act3.md §6.7
#[test]
fn pick_up() {
    let (mut ctl, mut f) = setup();
    f.f.item_codes.insert(ITEM, J34);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_PICKED_UP, Some(ITEM), P1),
    );
    let r = ctl.record(C).unwrap();
    assert_eq!((r.state, r.status), (1, 1));
    assert_eq!(f.log(), vec!["sound 1 72"]);
    assert!(has(&f, P1, 6));
    assert_eq!(to_5d(&f), vec![P1]);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_PICKED_UP, Some(ITEM), P1),
    );
    assert_eq!(f.log().len(), 1);
    f.f.item_codes.insert(ITEM, G34);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_PICKED_UP, Some(ITEM), P1),
    );
    let r = ctl.record(C).unwrap();
    assert_eq!((r.state, r.status), (3, 3));
    // Lacking 20.0 and 20.1 only.
    for b in [0, 1] {
        let (mut ctl, mut f) = setup();
        f.f.item_codes.insert(ITEM, J34);
        bits(&mut f, P1, S, &[b]);
        call(
            &mut ctl,
            &mut f,
            C,
            ev(event::ITEM_PICKED_UP, Some(ITEM), P1),
        );
        assert_eq!(ctl.record(C).unwrap().state, 0);
        assert!(f.log().is_empty());
    }
}

// Covers: specs/world/quests-act3.md §6.7
#[test]
fn leave_and_join() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    xm(&mut ctl).held = 1;
    xm(&mut ctl).dropped = true;
    ctl.record_mut(C).unwrap().state = 3;
    // Event 9: count 0 → +0x15, status 6 to all.
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_DROPPED_WITH_QUEST_ITEM, Some(ITEM), P1),
    );
    assert_eq!(x(&ctl).held, 0);
    assert!(x(&ctl).holder_left);
    assert_eq!(ctl.record(C).unwrap().status, 6);
    assert_eq!(to_5d(&f), vec![P1, P2]);
    // Event 14 holding the bird: count 1 → status 3 to all.
    f.f.sent.clear();
    f.p(P2).items.push(G34);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_JOINED_GAME, Some(P2), P2),
    );
    assert_eq!(x(&ctl).held, 1);
    assert!(!x(&ctl).holder_left);
    assert_eq!(ctl.record(C).unwrap().status, 3);
    assert_eq!(to_5d(&f), vec![P1, P2]);
    // Holding only the figurine → status 1.
    let (mut ctl, mut f) = setup();
    xm(&mut ctl).holder_left = true;
    f.p(P1).items.push(J34);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_JOINED_GAME, Some(P1), P1),
    );
    assert_eq!(ctl.record(C).unwrap().status, 1);
    // Holding both: count 2 → nothing.
    let (mut ctl, mut f) = setup();
    xm(&mut ctl).holder_left = true;
    f.p(P1).items = vec![J34, G34];
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_JOINED_GAME, Some(P1), P1),
    );
    assert_eq!(x(&ctl).held, 2);
    assert!(x(&ctl).holder_left);
    assert!(f.f.sent.is_empty());
}

// Covers: specs/world/quests-act3.md §6.7
#[test]
fn leave_guards() {
    // State ≥ 4, or no drop yet, or intro: count only.
    for case in 0..3 {
        let (mut ctl, mut f) = setup();
        xm(&mut ctl).held = 1;
        xm(&mut ctl).dropped = case != 1;
        let r = ctl.record_mut(C).unwrap();
        r.state = if case == 0 { 4 } else { 1 };
        r.not_intro = case != 2;
        call(
            &mut ctl,
            &mut f,
            C,
            ev(event::PLAYER_DROPPED_WITH_QUEST_ITEM, Some(ITEM), P1),
        );
        assert_eq!(x(&ctl).held, 0);
        assert!(!x(&ctl).holder_left, "case {case}");
        assert!(f.f.sent.is_empty());
    }
}

// Covers: specs/world/quests-act3.md §6.7
#[test]
fn level_change_and_leave_game_lists() {
    let (mut ctl, mut f) = setup();
    let i = idx(&ctl);
    ctl.records[i].guids.add(1);
    ctl.records[i].guids.add(2);
    let lv = |old: u32| EventArgs {
        event: event::CHANGED_LEVEL,
        target: Some(P1),
        player: Some(P1),
        a: old,
        b: 76,
    };
    call(&mut ctl, &mut f, C, lv(74));
    assert_eq!(ctl.records[i].guids.0, vec![1, 2]);
    call(&mut ctl, &mut f, C, lv(75));
    assert_eq!(ctl.records[i].guids.0, vec![2]);
    // Event 10: list remove.
    f.add_player(P2, 75);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_LEAVES_GAME, Some(P2), P2),
    );
    assert!(ctl.records[i].guids.0.is_empty());
}

// ------------------------------------------------------------ §6.8

fn start(ctl: &mut QuestControl, f: &mut Fake3) -> (u8, u8) {
    call(ctl, f, C, ev(event::PLAYER_STARTED_GAME, Some(P1), P1));
    let r = ctl.record(C).unwrap();
    (r.state, r.status)
}

// Covers: specs/world/quests-act3.md §6.8
#[test]
fn game_start() {
    for b in [0, 1] {
        let (mut ctl, mut f) = setup();
        bits(&mut f, P1, S, &[b]);
        assert_eq!(start(&mut ctl, &mut f), (0, 0));
        assert!(!ctl.record(C).unwrap().not_intro);
        assert!(!x(&ctl).to_drop && !x(&ctl).may_choose);
    }
    let (mut ctl, mut f) = setup();
    f.p(P1).items.push(G34);
    assert_eq!(start(&mut ctl, &mut f), (3, 3));
    assert!(!x(&ctl).to_drop && !x(&ctl).may_choose);
    let (mut ctl, mut f) = setup();
    f.p(P1).items.push(G34);
    bits(&mut f, P1, S, &[4]);
    assert_eq!(start(&mut ctl, &mut f), (3, 4));
    let (mut ctl, mut f) = setup();
    f.p(P1).items.push(J34);
    assert_eq!(start(&mut ctl, &mut f), (1, 1));
    assert!(!x(&ctl).to_drop && !x(&ctl).may_choose);
    let (mut ctl, mut f) = setup();
    f.p(P1).items.push(J34);
    bits(&mut f, P1, S, &[2]);
    assert_eq!(start(&mut ctl, &mut f), (2, 2));
    // Nothing → nothing.
    let (mut ctl, mut f) = setup();
    assert_eq!(start(&mut ctl, &mut f), (0, 0));
    assert!(x(&ctl).to_drop && x(&ctl).may_choose);
    assert!(ctl.record(C).unwrap().not_intro);
}

// ------------------------------------------------------------ §6.9

// Covers: specs/world/quests-act3.md §6.9
#[test]
fn status_function() {
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[1]);
    assert_eq!(status_of(&ctl, &mut f, P1), 0);
    let fresh = |f: &mut Fake3, bs: &[u8]| {
        let d = usize::from(f.f.difficulty);
        f.p(P1).quests.flags[d] = QuestFlags::default();
        bits(f, P1, 15, &[0]);
        bits(f, P1, S, bs);
    };
    fresh(&mut f, &[1, 0]);
    assert_eq!(status_of(&ctl, &mut f, P1), 5);
    fresh(&mut f, &[0]);
    assert_eq!(status_of(&ctl, &mut f, P1), 11);
    fresh(&mut f, &[0, 13]);
    assert_eq!(status_of(&ctl, &mut f, P1), 13);
    fresh(&mut f, &[]);
    f.p(P1).items = vec![G34, J34];
    assert_eq!(status_of(&ctl, &mut f, P1), 3);
    fresh(&mut f, &[4]);
    assert_eq!(status_of(&ctl, &mut f, P1), 4);
    f.p(P1).items = vec![J34];
    fresh(&mut f, &[]);
    assert_eq!(status_of(&ctl, &mut f, P1), 1);
    fresh(&mut f, &[2]);
    assert_eq!(status_of(&ctl, &mut f, P1), 2);
    f.p(P1).items.clear();
    // The status byte: ≤ 5 as is; > 5 → 6 + (game type ≠ 3).
    ctl.record_mut(C).unwrap().status = 4;
    assert_eq!(status_of(&ctl, &mut f, P1), 4);
    ctl.record_mut(C).unwrap().status = 6;
    assert_eq!(status_of(&ctl, &mut f, P1), 7);
    f.f.game_type = 3;
    assert_eq!(status_of(&ctl, &mut f, P1), 6);
    ctl.record_mut(C).unwrap().not_intro = false;
    assert_eq!(status_of(&ctl, &mut f, P1), 0);
}

// Covers: specs/world/quests-act3.md §6.1
#[test]
fn extra_data_at_init() {
    // +0x01 (a boss may be chosen) and +0x0C (figurine still to drop) are
    // 1 at init; every other field is 0.
    let (ctl, _) = control();
    assert_eq!(
        x(&ctl),
        &act3::q4::Extra {
            may_choose: true,
            to_drop: true,
            ..act3::q4::Extra::default()
        }
    );
}
