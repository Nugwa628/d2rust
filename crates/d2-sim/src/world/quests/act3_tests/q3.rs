// Spec: specs/world/quests-act3.md §5 (A3Q3 Blade of the Old Religion)
//! Chain 17 callback by callback: chat, messages, the ring, the decoy,
//! its timer and boss, the altar, level changes, pick-up, join / leave,
//! game start and the status function.

use super::*;

const C: u8 = 17;
const S: u8 = 19;
const G33: [u8; 4] = *b"g33 ";
const P3: UnitId = UnitId(3);
const DECOY: UnitId = UnitId(0x40);
const ALTAR: UnitId = UnitId(0x41);
const BOSS: UnitId = UnitId(0x42);
const ITEM: UnitId = UnitId(0x43);
/// The decoy's and the altar's init records (`quests-act3-2.md` §11.7).
const DECOY_AT: act3::InitPoint = act3::InitPoint {
    room: RoomId(7),
    x: 100,
    y: 200,
};
const ALTAR_AT: act3::InitPoint = act3::InitPoint {
    room: RoomId(3),
    x: 300,
    y: 400,
};

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

fn x(ctl: &QuestControl) -> &act3::q3::Extra {
    &ctl.record(C).unwrap().extra.act3.q3
}

fn xm(ctl: &mut QuestControl) -> &mut act3::q3::Extra {
    &mut ctl.record_mut(C).unwrap().extra.act3.q3
}

fn ev(event: u8, target: Option<UnitId>, player: UnitId) -> EventArgs {
    EventArgs {
        event,
        target,
        player: Some(player),
        ..EventArgs::default()
    }
}

/// Event 11 from `p` to the NPC unit (class from the fake).
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
    (control().0, Fake3::new())
}

// ------------------------------------------------------------ §5.2, §5.7

// Covers: specs/world/quests-act3.md §5.2, §5.4, §5.7
#[test]
fn hratli_start_and_chat_end_iterate() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    f.add_player(P3, 75);
    f.p(P1).items.push(G33);
    bits(&mut f, P2, S, &[0]);
    msg(&mut ctl, &mut f, P3, HRATLI_U, 571);
    assert_eq!(ctl.record(C).unwrap().state, 2);
    assert!(x(&ctl).hratli_started);
    // A chat end with another NPC does nothing.
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(ORMUS_U), P3),
    );
    assert!(x(&ctl).hratli_started);
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(HRATLI_U), P3),
    );
    let r = ctl.record(C).unwrap();
    assert_eq!((r.status, r.flags), (2, 0));
    assert!(!x(&ctl).hratli_started);
    // Status 2 to all: P2 has 19.0 and 19.15 clear is false → no send.
    let to: Vec<UnitId> = sent_5d(&f).iter().map(|m| m.0).collect();
    assert_eq!(to, vec![P1, P3]);
    // The flag iterate: holder → 19.5 only; 19.0 → nothing; state 2 → 19.3.
    assert!(has(&f, P1, 5) && !has(&f, P1, 3));
    assert_eq!(f.flags(P2).word(S), 1);
    assert!(has(&f, P3, 3) && !has(&f, P3, 5));
    // Edge case 6: callback 2 stays set; a second chat end does nothing.
    assert!(ctl.record(C).unwrap().has_callback(event::NPC_DEACTIVATE));
    f.f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(HRATLI_U), P3),
    );
    assert!(sent_5d(&f).is_empty());
}

// Covers: specs/world/quests-act3.md §5.2, §5.6
#[test]
fn flag_iterate_after_the_drop() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    f.add_player(P3, 75);
    bits(&mut f, P2, S, &[3]);
    bits(&mut f, P3, S, &[6]);
    xm(&mut ctl).boss_spawned = true;
    f.special = true;
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::MONSTER_KILLED, Some(BOSS), P1),
    );
    assert!(x(&ctl).gidbinn_dropped);
    // Dropped: 19.4, and 19.2 unless 19.3.
    assert!(has(&f, P1, 4) && has(&f, P1, 2));
    assert!(has(&f, P2, 4) && !has(&f, P2, 2));
    // 19.6 → skipped.
    assert!(!has(&f, P3, 4));
}

// ------------------------------------------------------------ §5.3

// Covers: specs/world/quests-act3.md §5.3
#[test]
fn chat_lists() {
    let (mut ctl, mut f) = setup();
    // State 0 → index −1: nothing.
    assert!(text(&mut ctl, &mut f, C, P1, HRATLI_U).is_empty());
    // States 1–3 → table states 0–2.
    ctl.record_mut(C).unwrap().state = 1;
    assert_eq!(text(&mut ctl, &mut f, C, P1, HRATLI_U), vec![(571, 0)]);
    ctl.record_mut(C).unwrap().state = 2;
    assert_eq!(text(&mut ctl, &mut f, C, P1, HRATLI_U), vec![(576, 2)]);
    ctl.record_mut(C).unwrap().state = 3;
    assert_eq!(text(&mut ctl, &mut f, C, P1, HRATLI_U), vec![(583, 2)]);
    // State > 3 → nothing.
    ctl.record_mut(C).unwrap().state = 4;
    assert!(text(&mut ctl, &mut f, C, P1, HRATLI_U).is_empty());
    // Holding without 19.5 → nothing.
    ctl.record_mut(C).unwrap().state = 1;
    f.p(P1).items.push(G33);
    assert!(text(&mut ctl, &mut f, C, P1, HRATLI_U).is_empty());
    // Holding with 19.5, 19.8 clear → 3.
    bits(&mut f, P1, S, &[5]);
    assert_eq!(text(&mut ctl, &mut f, C, P1, ORMUS_U), vec![(587, 0)]);
    // 19.5 without the item → 4.
    f.p(P1).items.clear();
    assert_eq!(text(&mut ctl, &mut f, C, P1, ALKOR_U), vec![(586, 2)]);
}

// Covers: specs/world/quests-act3.md §5.3
#[test]
fn chat_after_handing_in() {
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[6]);
    // Asheara with 19.7 clear → 5.
    assert_eq!(text(&mut ctl, &mut f, C, P1, ASHEARA_U), vec![(589, 0)]);
    // Ormus with 19.8 clear → 6 (before "any NPC but Asheara → 5").
    assert_eq!(text(&mut ctl, &mut f, C, P1, ORMUS_U), vec![(593, 0)]);
    // Any other NPC with 19.7 clear → 5.
    assert_eq!(text(&mut ctl, &mut f, C, P1, ALKOR_U), vec![(586, 2)]);
    // Ring taken, mercenary not: Ormus falls to "any NPC but Asheara" → 5.
    bits(&mut f, P1, S, &[8]);
    assert!(text(&mut ctl, &mut f, C, P1, ORMUS_U).is_empty()); // no Ormus row in 5
    assert_eq!(text(&mut ctl, &mut f, C, P1, MESHIF2_U), vec![(588, 2)]);
    // Mercenary taken, ring not: Asheara → 6 (no Asheara row: nothing),
    // Ormus → 6.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[6, 7]);
    assert!(text(&mut ctl, &mut f, C, P1, ASHEARA_U).is_empty());
    assert_eq!(text(&mut ctl, &mut f, C, P1, ORMUS_U), vec![(593, 0)]);
    // Both taken → nothing.
    bits(&mut f, P1, S, &[8]);
    assert!(text(&mut ctl, &mut f, C, P1, ORMUS_U).is_empty());
    assert!(text(&mut ctl, &mut f, C, P1, MESHIF2_U).is_empty());
}

// Covers: specs/world/quests-act3.md §5.3
#[test]
fn chat_when_done_needs_the_guid() {
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[0]);
    assert!(text(&mut ctl, &mut f, C, P1, ALKOR_U).is_empty());
    let i = idx(&ctl);
    ctl.records[i].guids.add(1);
    assert_eq!(text(&mut ctl, &mut f, C, P1, ALKOR_U), vec![(586, 2)]);
}

// Covers: specs/world/quests-act3.md §5.3
#[test]
fn wants_to_talk() {
    let (mut ctl, mut f) = setup();
    let i = idx(&ctl);
    let act = |ctl: &QuestControl, f: &mut Fake3, n: u16| act3::active(ctl, f, i, P1, n);
    assert!(!act(&ctl, &mut f, act3::npc::HRATLI));
    ctl.records[i].state = 1;
    assert!(act(&ctl, &mut f, act3::npc::HRATLI));
    assert!(!act(&ctl, &mut f, act3::npc::ORMUS));
    f.p(P1).items.push(G33);
    assert!(!act(&ctl, &mut f, act3::npc::HRATLI));
    assert!(act(&ctl, &mut f, act3::npc::ORMUS));
    f.p(P1).items.clear();
    bits(&mut f, P1, S, &[6]);
    assert!(act(&ctl, &mut f, act3::npc::ORMUS));
    assert!(act(&ctl, &mut f, act3::npc::ASHEARA));
    assert!(!act(&ctl, &mut f, act3::npc::ALKOR));
    bits(&mut f, P1, S, &[7, 8]);
    assert!(!act(&ctl, &mut f, act3::npc::ORMUS));
    assert!(!act(&ctl, &mut f, act3::npc::ASHEARA));
    // 19.0 → never.
    let (ctl, mut f) = setup();
    f.p(P1).items.push(G33);
    bits(&mut f, P1, S, &[0]);
    assert!(!act3::active(&ctl, &mut f, i, P1, act3::npc::ORMUS));
}

// ------------------------------------------------------------ §5.4, §5.5

// Covers: specs/world/quests-act3.md §5.4
#[test]
fn ormus_takes_the_gidbinn() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    f.add_player(P3, 75);
    let p4 = UnitId(4);
    let p5 = UnitId(5);
    f.add_player(p4, 75);
    f.add_player(p5, 1);
    f.p(p5).act = Some(0);
    f.p(P1).items.push(G33);
    f.p(P3).items.push(G33);
    bits(&mut f, P1, S, &[5]);
    bits(&mut f, p4, S, &[7]);
    f.f.party.insert(P1, vec![P1, P2, p5]);
    xm(&mut ctl).held = 2;
    ctl.record_mut(C).unwrap().state = 4;
    msg(&mut ctl, &mut f, P1, ORMUS_U, 587);
    assert!(x(&ctl).brought);
    assert_eq!(x(&ctl).held, 1);
    assert!(!f.p(P1).items.contains(&G33));
    assert!(f.log().contains(&"delete g33 ".to_string()));
    assert!(has(&f, P1, 6));
    // Party member in Act III → 19.6; p5 (Act I) no 19.6.
    assert!(has(&f, P2, 6));
    assert!(!has(&f, p5, 6));
    // For each player lacking the bits and not holding: 19.14.
    assert!(has(&f, p5, 14));
    assert!(!has(&f, P3, 14)); // holds the Gidbinn
    assert!(!has(&f, p4, 14)); // has 19.7
    assert!(!has(&f, P1, 14) && !has(&f, P2, 14));
    // Nothing sent.
    assert!(f.f.sent.is_empty());
    assert!(ctl.game.get(S, 13));
    let r = ctl.record(C).unwrap();
    assert_eq!((r.status, r.state), (13, 5));
}

// Covers: specs/world/quests-act3.md §5.4
#[test]
fn ormus_587_without_the_gidbinn() {
    let (mut ctl, mut f) = setup();
    msg(&mut ctl, &mut f, P1, ORMUS_U, 587);
    // "Then (always)": game 19.13, status 13, state 5.
    assert!(!x(&ctl).brought && !has(&f, P1, 6));
    assert!(ctl.game.get(S, 13));
    let r = ctl.record(C).unwrap();
    assert_eq!((r.status, r.state), (13, 5));
    // Intro: only the silent status.
    let (mut ctl, mut f) = setup();
    ctl.record_mut(C).unwrap().not_intro = false;
    msg(&mut ctl, &mut f, P1, ORMUS_U, 587);
    assert!(!ctl.game.get(S, 13));
    let r = ctl.record(C).unwrap();
    assert_eq!((r.status, r.state), (13, 0));
    // 19.0 → nothing.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[0]);
    msg(&mut ctl, &mut f, P1, ORMUS_U, 587);
    assert_eq!(ctl.record(C).unwrap().status, 0);
    assert!(!ctl.game.get(S, 13));
}

// Covers: specs/world/quests-act3.md §5.4, §5.5
#[test]
fn ormus_ring_on_nightmare() {
    // Test vector: Ormus 593 on nightmare, 19.6 set, 19.8 clear.
    let (mut ctl, mut f) = setup();
    f.f.difficulty = 1;
    bits(&mut f, P1, S, &[6]);
    msg(&mut ctl, &mut f, P1, ORMUS_U, 593);
    assert!(has(&f, P1, 8));
    assert_eq!(f.log(), vec!["reward rin  35 6".to_string()]);
    assert!(!has(&f, P1, 0) && !has(&f, P1, 13));
    // A second 593 does nothing.
    msg(&mut ctl, &mut f, P1, ORMUS_U, 593);
    assert_eq!(f.log().len(), 1);
}

// Covers: specs/world/quests-act3.md §5.5
#[test]
fn ring_level_by_difficulty() {
    for (d, level) in [(0, 21), (1, 35), (2, 75)] {
        let (mut ctl, mut f) = setup();
        f.f.difficulty = d;
        bits(&mut f, P1, S, &[6]);
        msg(&mut ctl, &mut f, P1, ORMUS_U, 593);
        assert_eq!(f.log(), vec![format!("reward rin  {level} 6")]);
    }
    // Without 19.6 → nothing.
    let (mut ctl, mut f) = setup();
    msg(&mut ctl, &mut f, P1, ORMUS_U, 593);
    assert!(f.log().is_empty() && !has(&f, P1, 8));
}

// Covers: specs/world/quests-act3.md §5.4
#[test]
fn both_rewards_finish_the_quest() {
    // Ring, then mercenary.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[6]);
    msg(&mut ctl, &mut f, P1, ORMUS_U, 593);
    msg(&mut ctl, &mut f, P1, ASHEARA_U, 589);
    assert!(f.log().contains(&"merc 252".to_string()));
    assert!(has(&f, P1, 7) && has(&f, P1, 0) && has(&f, P1, 13));
    assert!(ctl.record(C).unwrap().guids.contains(1));
    // Mercenary, then ring.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[6]);
    msg(&mut ctl, &mut f, P1, ASHEARA_U, 589);
    assert!(has(&f, P1, 7) && !has(&f, P1, 0));
    assert!(ctl.record(C).unwrap().guids.0.is_empty());
    msg(&mut ctl, &mut f, P1, ORMUS_U, 593);
    assert!(has(&f, P1, 0) && has(&f, P1, 13));
    assert!(ctl.record(C).unwrap().guids.contains(1));
    // 589 needs only 19.7 clear; a second one does nothing.
    let (mut ctl, mut f) = setup();
    msg(&mut ctl, &mut f, P1, ASHEARA_U, 589);
    msg(&mut ctl, &mut f, P1, ASHEARA_U, 589);
    assert_eq!(f.log(), vec!["merc 252".to_string()]);
}

// ------------------------------------------------------------ §5.6

fn decoy(f: &mut Fake3) {
    f.f.objects.insert(DECOY, (0x40, 252, 0));
}

// Covers: specs/world/quests-act3.md §5.6
#[test]
fn decoy_operate_starts_the_timer() {
    let (mut ctl, mut f) = setup();
    decoy(&mut f);
    f.f.frame = 50;
    act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
    assert_eq!(ctl.record(C).unwrap().state, 1);
    assert_eq!(f.log(), vec!["mode 64 1", "event1 64 66"]);
    assert!(x(&ctl).decoy_active && x(&ctl).timer);
    assert_eq!(ctl.timers.len(), 1);
    let t = ctl.timers[0];
    assert_eq!(
        (t.func, t.chain, t.period),
        (TimerFn::Act3(act3::Timer::GidbinnBoss), C, 7)
    );
    // Mode ≠ 0 → nothing.
    act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(f.log().len(), 2);
    // Mode 0 again with the timer still there: no second timer; state
    // stays.
    f.f.objects.get_mut(&DECOY).unwrap().2 = 0;
    ctl.record_mut(C).unwrap().state = 3;
    act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(ctl.record(C).unwrap().state, 3);
}

// Covers: specs/world/quests-act3.md §5.6
#[test]
fn decoy_operate_refused_and_intro() {
    for b in [0, 7, 8] {
        let (mut ctl, mut f) = setup();
        decoy(&mut f);
        bits(&mut f, P1, S, &[b]);
        act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
        assert_eq!(f.log(), vec!["sound 1 19"]);
        assert!(ctl.timers.is_empty() && !x(&ctl).decoy_active);
    }
    let (mut ctl, mut f) = setup();
    decoy(&mut f);
    ctl.record_mut(C).unwrap().not_intro = false;
    act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
    assert!(f.log().is_empty() && ctl.timers.is_empty());
}

// Covers: specs/world/quests-act3.md §5.6
#[test]
fn decoy_init() {
    let (mut ctl, mut f) = setup();
    decoy(&mut f);
    act3::decoy_init(&mut ctl, &mut f, DECOY, DECOY_AT);
    assert!(x(&ctl).decoy_known);
    assert_eq!((x(&ctl).decoy_x, x(&ctl).decoy_y), (100, 200));
    assert!(f.log().is_empty());
    // Activated, no boss → spawn in the object's room.
    xm(&mut ctl).decoy_active = true;
    f.f.spawns.push(Some(BOSS));
    act3::decoy_init(&mut ctl, &mut f, DECOY, DECOY_AT);
    assert_eq!(f.log(), vec!["spawn in room 7 407"]);
    let e = x(&ctl);
    assert!(e.boss_spawned && !e.decoy_active && !e.boss_spawning);
    assert_eq!(e.boss_guid, BOSS.0);
    // Init spawns without installing the kill callback.
    assert!(!ctl.record(C).unwrap().has_callback(event::MONSTER_KILLED));
    // Intro → mode 2.
    let (mut ctl, mut f) = setup();
    decoy(&mut f);
    ctl.record_mut(C).unwrap().not_intro = false;
    act3::decoy_init(&mut ctl, &mut f, DECOY, DECOY_AT);
    assert_eq!(f.log(), vec!["mode 64 2"]);
    assert!(!x(&ctl).decoy_known);
}

fn run_timer(ctl: &mut QuestControl, f: &mut Fake3) {
    for _ in 0..8 {
        ctl.update(f);
    }
}

// Covers: specs/world/quests-act3.md §5.6
#[test]
fn boss_timer_spawns_near_a_player() {
    let (mut ctl, mut f) = setup();
    decoy(&mut f);
    act3::decoy_init(&mut ctl, &mut f, DECOY, DECOY_AT);
    act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
    f.room_covering = Some(RoomId(9));
    f.player_in_rooms = true;
    f.f.spawns.push(Some(BOSS));
    f.f.log.clear();
    for _ in 0..7 {
        ctl.update(&mut f);
    }
    assert_eq!(ctl.timers.len(), 1);
    ctl.update(&mut f);
    assert!(ctl.timers.is_empty());
    assert_eq!(
        f.log(),
        vec!["room covering 100 200", "spawn in room 9 407"]
    );
    assert!(!x(&ctl).timer && x(&ctl).boss_spawned);
    assert!(ctl.record(C).unwrap().has_callback(event::MONSTER_KILLED));
}

// Covers: specs/world/quests-act3.md §5.6, §edge-cases-original-bugs r4
#[test]
fn boss_timer_tries_once() {
    let (mut ctl, mut f) = setup();
    decoy(&mut f);
    act3::decoy_init(&mut ctl, &mut f, DECOY, DECOY_AT);
    act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
    f.room_covering = Some(RoomId(9));
    f.player_in_rooms = false;
    f.f.log.clear();
    run_timer(&mut ctl, &mut f);
    // Removed after one attempt; no boss; no callback 8.
    assert!(ctl.timers.is_empty());
    assert_eq!(f.log(), vec!["room covering 100 200"]);
    assert!(!x(&ctl).timer && !x(&ctl).boss_spawned && x(&ctl).decoy_active);
    assert!(!ctl.record(C).unwrap().has_callback(event::MONSTER_KILLED));
    // A player near later changes nothing until the decoy room's init.
    f.player_in_rooms = true;
    run_timer(&mut ctl, &mut f);
    assert!(!x(&ctl).boss_spawned);
    f.f.spawns.push(Some(BOSS));
    act3::decoy_init(&mut ctl, &mut f, DECOY, DECOY_AT);
    assert!(x(&ctl).boss_spawned);
}

// Covers: specs/world/quests-act3.md §5.6
#[test]
fn boss_timer_preconditions() {
    // Decoy not initialised / no Act III / no room: no spawn, removed.
    for case in 0..3 {
        let (mut ctl, mut f) = setup();
        decoy(&mut f);
        if case != 0 {
            act3::decoy_init(&mut ctl, &mut f, DECOY, DECOY_AT);
        }
        act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
        f.act3 = case != 1;
        f.room_covering = (case != 2).then_some(RoomId(9));
        f.player_in_rooms = true;
        f.f.spawns.push(Some(BOSS));
        f.f.log.clear();
        run_timer(&mut ctl, &mut f);
        assert!(ctl.timers.is_empty());
        assert!(!x(&ctl).boss_spawned, "case {case}");
        assert!(!f.log().iter().any(|l| l.starts_with("spawn")));
    }
    // Spawn fails → nothing changes but +0x04 returns to 0; callback 8
    // is still installed.
    let (mut ctl, mut f) = setup();
    decoy(&mut f);
    act3::decoy_init(&mut ctl, &mut f, DECOY, DECOY_AT);
    act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
    f.room_covering = Some(RoomId(9));
    f.player_in_rooms = true;
    run_timer(&mut ctl, &mut f);
    let e = x(&ctl);
    assert!(!e.boss_spawned && e.decoy_active && !e.boss_spawning);
    assert!(ctl.record(C).unwrap().has_callback(event::MONSTER_KILLED));
}

// Covers: specs/world/quests-act3.md §5.6
#[test]
fn boss_kill_drops_the_gidbinn() {
    let (mut ctl, mut f) = setup();
    let i = idx(&ctl);
    ctl.records[i].callbacks |= 1 << event::MONSTER_KILLED;
    xm(&mut ctl).boss_spawned = true;
    f.f.monsters.insert(
        BOSS,
        (
            0x42,
            407,
            UnitKind::Monster {
                class: 407,
                superunique: None,
                owner: None,
            },
        ),
    );
    f.f.chains.insert(BOSS, QuestChain(vec![C]));
    // Not special → nothing.
    ctl.monster_killed(&mut f, BOSS, Some(P1));
    assert!(f.log().is_empty());
    f.special = true;
    // Drop fails → +0x02 := 0, callback stays.
    f.drops = vec![false];
    ctl.monster_killed(&mut f, BOSS, Some(P1));
    assert_eq!(f.log(), vec!["qdrop 66 g33  2 false"]);
    assert!(!x(&ctl).boss_spawned && !x(&ctl).gidbinn_dropped);
    assert!(ctl.records[i].has_callback(event::MONSTER_KILLED));
    // Needs +0x02.
    ctl.monster_killed(&mut f, BOSS, Some(P1));
    assert_eq!(f.log().len(), 1);
    // Created → +0x00, iterate, callback 8 cleared.
    xm(&mut ctl).boss_spawned = true;
    ctl.monster_killed(&mut f, BOSS, Some(P1));
    assert!(x(&ctl).gidbinn_dropped && has(&f, P1, 4) && has(&f, P1, 2));
    assert!(!ctl.records[i].has_callback(event::MONSTER_KILLED));
    // Intro → nothing.
    let (mut ctl, mut f) = setup();
    xm(&mut ctl).boss_spawned = true;
    ctl.record_mut(C).unwrap().not_intro = false;
    f.special = true;
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::MONSTER_KILLED, Some(BOSS), P1),
    );
    assert!(f.log().is_empty());
}

// ------------------------------------------------------------ §5.7

// Covers: specs/world/quests-act3.md §5.7
#[test]
fn altar_and_ormus() {
    let (mut ctl, mut f) = setup();
    f.f.objects.insert(ALTAR, (0x41, 251, 0));
    act3::altar_init(&mut ctl, &mut f, ALTAR, ALTAR_AT);
    assert_eq!(f.log(), vec!["mode 65 0"]);
    let e = x(&ctl);
    assert_eq!((e.altar_guid, e.altar_x, e.altar_y), (0x41, 300, 400));
    assert_eq!(act3::altar_position(&ctl), None);
    // Ormus' chat end with +0x07 → +0x06.
    xm(&mut ctl).brought = true;
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(HRATLI_U), P1),
    );
    assert!(x(&ctl).brought);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::NPC_DEACTIVATE, Some(ORMUS_U), P1),
    );
    assert!(!x(&ctl).brought && x(&ctl).altar_ready);
    assert_eq!(act3::altar_position(&ctl), Some((300, 400)));
    f.f.frame = 10;
    f.f.log.clear();
    act3::activate_altar(&mut ctl, &mut f);
    assert_eq!(f.log(), vec!["mode 65 1", "event1 65 27"]);
    assert!(!x(&ctl).altar_ready);
    assert_eq!(x(&ctl).altar_mode, 2);
    assert_eq!(act3::altar_position(&ctl), None);
    // A later init takes mode +0x2C.
    f.f.log.clear();
    act3::altar_init(&mut ctl, &mut f, ALTAR, ALTAR_AT);
    assert_eq!(f.log(), vec!["mode 65 2"]);
}

// ------------------------------------------------------------ §5.8

fn level(ctl: &mut QuestControl, f: &mut Fake3, p: UnitId, old: u32, new: u32) {
    let args = EventArgs {
        event: event::CHANGED_LEVEL,
        target: Some(p),
        player: Some(p),
        a: old,
        b: new,
    };
    call(ctl, f, C, args);
}

// Covers: specs/world/quests-act3.md §5.8
#[test]
fn level_changes() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    level(&mut ctl, &mut f, P1, 77, 78);
    assert_eq!(ctl.record(C).unwrap().state, 1);
    // Leaving the Docks in state 1: quick remove only.
    let i = idx(&ctl);
    ctl.records[i].guids.add(1);
    ctl.records[i].guids.add(2);
    level(&mut ctl, &mut f, P1, 75, 76);
    assert_eq!(ctl.records[i].guids.0, vec![2]);
    assert_eq!(ctl.records[i].state, 1);
    // State 2, lacking 19.0 / 19.15 → state 3, the mover's iterate.
    ctl.records[i].state = 2;
    bits(&mut f, P1, S, &[15]);
    level(&mut ctl, &mut f, P1, 75, 76);
    assert_eq!(ctl.records[i].state, 2);
    level(&mut ctl, &mut f, P2, 75, 76);
    assert_eq!(ctl.records[i].state, 3);
    // The iterate runs after state := 3: no 19.3 for a player without
    // the item or the drop.
    assert!(!has(&f, P2, 3));
    // Flayer Jungle in state ≠ 0 or intro: nothing.
    level(&mut ctl, &mut f, P2, 77, 78);
    assert_eq!(ctl.records[i].state, 3);
    let (mut ctl, mut f) = setup();
    ctl.record_mut(C).unwrap().not_intro = false;
    level(&mut ctl, &mut f, P1, 77, 78);
    assert_eq!(ctl.record(C).unwrap().state, 0);
}

// Covers: specs/world/quests-act3.md §5.8
#[test]
fn level_change_iterate_with_the_drop() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    ctl.record_mut(C).unwrap().state = 2;
    xm(&mut ctl).gidbinn_dropped = true;
    level(&mut ctl, &mut f, P2, 75, 76);
    assert!(has(&f, P2, 4) && has(&f, P2, 2));
    // Only the moving player.
    assert!(!has(&f, P1, 4));
}

// Covers: specs/world/quests-act3.md §5.8, §edge-cases-original-bugs r5
#[test]
fn pick_up_sends_status_4_then_3() {
    let (mut ctl, mut f) = setup();
    f.add_player(P2, 75);
    f.f.item_codes.insert(ITEM, G33);
    f.p(P1).items.push(G33);
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_PICKED_UP, Some(ITEM), P1),
    );
    // Two rounds of 0x5D to every player: status 4, then status 3.
    let to: Vec<UnitId> = sent_5d(&f).iter().map(|m| m.0).collect();
    assert_eq!(to, vec![P1, P2, P1, P2]);
    let r = ctl.record(C).unwrap();
    assert_eq!((r.state, r.status), (4, 3));
    assert_eq!(f.log(), vec!["sound 1 65"]);
    assert!(has(&f, P1, 9) && has(&f, P1, 5));
    // Second pick-up: no sound.
    f.f.log.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_PICKED_UP, Some(ITEM), P1),
    );
    assert!(f.log().is_empty());
}

// Covers: specs/world/quests-act3.md §5.8
#[test]
fn pick_up_other_item_or_intro_only_iterates() {
    let (mut ctl, mut f) = setup();
    f.f.item_codes.insert(ITEM, *b"j34 ");
    xm(&mut ctl).gidbinn_dropped = true;
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_PICKED_UP, Some(ITEM), P1),
    );
    assert!(sent_5d(&f).is_empty() && f.log().is_empty());
    assert_eq!(ctl.record(C).unwrap().state, 0);
    assert!(has(&f, P1, 4));
    let (mut ctl, mut f) = setup();
    f.f.item_codes.insert(ITEM, G33);
    f.p(P1).items.push(G33);
    ctl.record_mut(C).unwrap().not_intro = false;
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::ITEM_PICKED_UP, Some(ITEM), P1),
    );
    assert!(sent_5d(&f).is_empty());
    assert!(has(&f, P1, 5));
}

// Covers: specs/world/quests-act3.md §5.8
#[test]
fn join_and_leave_counts() {
    let (mut ctl, mut f) = setup();
    f.f.item_codes.insert(ITEM, G33);
    f.p(P1).items.push(G33);
    xm(&mut ctl).held = 1;
    xm(&mut ctl).gidbinn_dropped = true;
    ctl.record_mut(C).unwrap().state = 4;
    // Leave with the Gidbinn: count 0 → +0x20.
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_DROPPED_WITH_QUEST_ITEM, Some(ITEM), P1),
    );
    assert_eq!(x(&ctl).held, 0);
    assert!(x(&ctl).holder_left);
    // Join holding it: count 1 → +0x20 := 0. Nothing sent.
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_JOINED_GAME, Some(P1), P1),
    );
    assert_eq!(x(&ctl).held, 1);
    assert!(!x(&ctl).holder_left);
    assert!(f.f.sent.is_empty());
    // State 5 → no +0x20.
    ctl.record_mut(C).unwrap().state = 5;
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_DROPPED_WITH_QUEST_ITEM, Some(ITEM), P1),
    );
    assert!(!x(&ctl).holder_left);
    // Another item → no count change.
    f.f.item_codes.insert(ITEM, *b"j34 ");
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_DROPPED_WITH_QUEST_ITEM, Some(ITEM), P1),
    );
    assert_eq!(x(&ctl).held, 0);
    // Join without it: no change.
    f.p(P1).items.clear();
    call(
        &mut ctl,
        &mut f,
        C,
        ev(event::PLAYER_JOINED_GAME, Some(P1), P1),
    );
    assert_eq!(x(&ctl).held, 0);
}

fn start(ctl: &mut QuestControl, f: &mut Fake3) -> (u8, u8) {
    call(ctl, f, C, ev(event::PLAYER_STARTED_GAME, Some(P1), P1));
    let r = ctl.record(C).unwrap();
    (r.state, r.status)
}

// Covers: specs/world/quests-act3.md §5.8
#[test]
fn game_start() {
    // 19.0 → +0x2C := 2 only.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[0]);
    assert_eq!(start(&mut ctl, &mut f), (0, 0));
    assert_eq!(x(&ctl).altar_mode, 2);
    // Holding → count, status 4, state 4, iterate (19.5).
    let (mut ctl, mut f) = setup();
    f.p(P1).items.push(G33);
    assert_eq!(start(&mut ctl, &mut f), (4, 4));
    assert_eq!(x(&ctl).held, 1);
    assert!(has(&f, P1, 5));
    // 19.5 alone → state 3, status 2.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[5]);
    assert_eq!(start(&mut ctl, &mut f), (3, 2));
    // 19.5 then 19.4 → state 2, status 2.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[5, 4]);
    assert_eq!(start(&mut ctl, &mut f), (2, 2));
    // 19.3 → state 2, status 2; 19.2 → state 3, status 1.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[3, 2]);
    assert_eq!(start(&mut ctl, &mut f), (2, 2));
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[2]);
    assert_eq!(start(&mut ctl, &mut f), (3, 1));
    let (mut ctl, mut f) = setup();
    assert_eq!(start(&mut ctl, &mut f), (0, 0));
    // 19.6: state 5; 19.7 clear → status 5.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[6]);
    assert_eq!(start(&mut ctl, &mut f), (5, 5));
    assert_eq!(x(&ctl).altar_mode, 2);
    // 19.6, 19.7 → status 6.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[6, 7]);
    assert_eq!(start(&mut ctl, &mut f), (5, 6));
    assert!(ctl.record(C).unwrap().not_intro);
    // 19.6, 19.7, 19.8 → 19.0 and not-intro := 0.
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, S, &[6, 7, 8]);
    assert_eq!(start(&mut ctl, &mut f), (5, 0));
    assert!(has(&f, P1, 0));
    assert!(!ctl.record(C).unwrap().not_intro);
}

// ------------------------------------------------------------ §5.9

// Covers: specs/world/quests-act3.md §5.9 text, §5.9 r5
#[test]
fn status_function() {
    let (ctl, mut f) = setup();
    // 15.0 clear → 0 whatever else.
    bits(&mut f, P1, S, &[0, 13]);
    assert_eq!(status_of(&ctl, &mut f, P1), 0);
    bits(&mut f, P1, 15, &[0]);
    assert_eq!(status_of(&ctl, &mut f, P1), 13);
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, 15, &[0]);
    bits(&mut f, P1, S, &[0]);
    assert_eq!(status_of(&ctl, &mut f, P1), 11);
    let fresh = |f: &mut Fake3, bs: &[u8]| {
        let d = usize::from(f.f.difficulty);
        f.p(P1).quests.flags[d] = QuestFlags::default();
        bits(f, P1, 15, &[0]);
        bits(f, P1, S, bs);
    };
    // Holds → 4.
    fresh(&mut f, &[6]);
    f.p(P1).items.push(G33);
    assert_eq!(status_of(&ctl, &mut f, P1), 4);
    f.p(P1).items.clear();
    // +0x20 → 7 + (game type 3).
    xm(&mut ctl).holder_left = true;
    assert_eq!(status_of(&ctl, &mut f, P1), 7);
    f.f.game_type = 3;
    assert_eq!(status_of(&ctl, &mut f, P1), 8);
    xm(&mut ctl).holder_left = false;
    // Test vector: 19.6, 19.7 set, 19.8 clear → 6.
    fresh(&mut f, &[6, 7]);
    assert_eq!(status_of(&ctl, &mut f, P1), 6);
    fresh(&mut f, &[6]);
    assert_eq!(status_of(&ctl, &mut f, P1), 5);
    fresh(&mut f, &[6, 8]);
    assert_eq!(status_of(&ctl, &mut f, P1), 5);
    fresh(&mut f, &[6, 7, 8]);
    assert_eq!(status_of(&ctl, &mut f, P1), 0);
    // 19.6 clear.
    fresh(&mut f, &[4]);
    assert_eq!(status_of(&ctl, &mut f, P1), 2);
    xm(&mut ctl).gidbinn_dropped = true;
    assert_eq!(status_of(&ctl, &mut f, P1), 3);
    fresh(&mut f, &[3]);
    assert_eq!(status_of(&ctl, &mut f, P1), 2);
    fresh(&mut f, &[2]);
    assert_eq!(status_of(&ctl, &mut f, P1), 1);
    fresh(&mut f, &[]);
    assert_eq!(status_of(&ctl, &mut f, P1), 0);
}

// Covers: specs/world/quests-act3.md §5.9 text
#[test]
fn status_reaches_the_0x5d() {
    let (mut ctl, mut f) = setup();
    bits(&mut f, P1, 15, &[0]);
    bits(&mut f, P1, S, &[6, 7]);
    let i = idx(&ctl);
    ctl.records[i].flags = 0x40;
    ctl.send_status(&mut f, P1, C).unwrap();
    assert_eq!(sent_5d(&f), vec![(P1, vec![0x5D, 17, 0x40, 6, 0, 0])]);
}

// Covers: specs/world/quests-act3.md §5.1
#[test]
fn extra_data_at_init() {
    // Chain 17's init only switches the record on: the extra data is 0.
    let (ctl, _) = control();
    assert_eq!(x(&ctl), &act3::q3::Extra::default());
    let r = &ctl.records[idx(&ctl)];
    assert!(r.active);
}
