// Spec: specs/world/quests-act5-2.md §7 (Test vectors, Edge cases)
//! A5Q5 Rite of Passage callback by callback on the quests' fake world:
//! the altar, the statues and the Ancients, the reset, the experience
//! reward, the doors and the hooks.

use super::super::super::tests::*;
use super::super::super::*;
use crate::units::RoomId;

const QUAL_U: UnitId = UnitId(0x40);
const LARZUK_U: UnitId = UnitId(0x41);
const STATUE_NPC_U: UnitId = UnitId(0x42);
const S474: UnitId = UnitId(0x50);
const S475: UnitId = UnitId(0x51);
const S476: UnitId = UnitId(0x52);
const ALTAR_U: UnitId = UnitId(0x53);
const DOOR_U: UnitId = UnitId(0x54);
const SUMMIT_DOOR_U: UnitId = UnitId(0x55);
const A540: UnitId = UnitId(0x60);
const A541: UnitId = UnitId(0x61);
const A542: UnitId = UnitId(0x62);

fn monster(f: &mut Fake, u: UnitId, class: u16, superunique: Option<u32>) {
    let kind = UnitKind::Monster {
        class: u32::from(class),
        superunique,
        owner: None,
    };
    f.monsters.insert(u, (u.0, class, kind));
}

/// P1 (level 25) on the summit in Act V; Qual-Kehk, Larzuk, a statue
/// NPC; the three statue objects, the altar and the doors.
fn fake() -> Fake {
    let mut f = Fake::new();
    f.p(P1).act = Some(4);
    f.p(P1).level = Some(120);
    f.p(P1).stats.insert(12, 25);
    monster(&mut f, QUAL_U, 515, None);
    monster(&mut f, LARZUK_U, 511, None);
    monster(&mut f, STATUE_NPC_U, 538, None);
    for (u, class) in [
        (S474, 474),
        (S475, 475),
        (S476, 476),
        (ALTAR_U, 546),
        (DOOR_U, 547),
        (SUMMIT_DOOR_U, 564),
    ] {
        f.objects.insert(u, (u.0, class, 0));
    }
    f
}

fn rec(ctl: &QuestControl) -> usize {
    ctl.find(35).unwrap()
}

fn ev(event: u8, target: Option<UnitId>, a: u32, b: u32) -> EventArgs {
    EventArgs {
        event,
        target,
        player: Some(P1),
        a,
        b,
    }
}

fn call(ctl: &mut QuestControl, f: &mut Fake, args: EventArgs) -> bool {
    let i = rec(ctl);
    super::callback(ctl, f, i, args, None)
}

fn text(ctl: &mut QuestControl, f: &mut Fake, n: UnitId) -> Vec<u16> {
    let mut list = TextList::new();
    let i = rec(ctl);
    super::callback(
        ctl,
        f,
        i,
        ev(event::NPC_ACTIVATE, Some(n), 0, 0),
        Some(&mut list),
    );
    list.iter().map(|e| e.0).collect()
}

fn status_msgs(f: &Fake, chain: u8) -> Vec<(UnitId, u8)> {
    f.sent
        .iter()
        .filter(|m| m.1[0] == 0x5D && m.1[1] == chain)
        .map(|m| (m.0, m.1[3]))
        .collect()
}

/// The statues' inits (63–65) and the altar's (72).
fn init_objects(ctl: &mut QuestControl, f: &mut Fake) {
    for (u, class) in [(S474, 474), (S475, 475), (S476, 476)] {
        super::statue_init(ctl, f, u, class);
    }
    super::altar_init(ctl, f, ALTAR_U);
}

/// The altar's message (20002) arms the statues; their events spawn
/// the three Ancients (474 → 542, 475 → 540, 476 → 541).
fn start_fight(ctl: &mut QuestControl, f: &mut Fake) {
    init_objects(ctl, f);
    call(
        ctl,
        f,
        ev(event::SCROLL_MESSAGE, Some(STATUE_NPC_U), 538, 20002),
    );
    for (s, a, class, su) in [
        (S474, A542, 542, 45),
        (S475, A540, 540, 43),
        (S476, A541, 541, 44),
    ] {
        monster(f, a, class, Some(su));
        f.chains.insert(a, QuestChain(vec![35]));
        f.pos.insert(a, (30, 40, RoomId(5)));
        f.a5_superuniques = vec![Some(a)];
        super::statue_event(ctl, f, s);
    }
}

fn thresholds(f: &mut Fake, pairs: &[(i32, u32)]) {
    for &(l, t) in pairs {
        f.a5_thresholds.insert(l, t);
    }
    f.a5_max_level = 99;
}

// Covers: specs/world/quests-act5-2.md §7.7
#[test]
fn reward_normal_exactly_one_level() {
    // Vector: normal, level 25, T(26) − T(25) = 40,000 (synthetic): A =
    // 40,000 → exactly one level.
    let mut f = fake();
    thresholds(&mut f, &[(25, 100_000), (26, 140_000), (27, 200_000)]);
    f.p(P1).stats.insert(13, 100_000);
    f.p(P1).stats.insert(30, 140_000);
    super::experience_reward(&mut f, P1);
    assert_eq!(f.sent_ids(), [0x28]);
    assert_eq!(f.log, ["level up 1"]);
    let s = &f.players[&P1].stats;
    assert_eq!(
        (s[&12], s[&13], s[&29], s[&30]),
        (26, 140_000, 40_000, 200_000)
    );
    // Mid-level: the cap is T(26) − T(25), not the gap.
    let mut f = fake();
    thresholds(&mut f, &[(25, 100_000), (26, 140_000), (27, 200_000)]);
    f.p(P1).stats.insert(13, 130_000);
    f.p(P1).stats.insert(30, 140_000);
    super::experience_reward(&mut f, P1);
    let s = &f.players[&P1].stats;
    assert_eq!((s[&12], s[&13]), (26, 170_000));
}

// Covers: specs/world/quests-act5-2.md §7.7
#[test]
fn reward_hell_capped_at_one_level() {
    // Vector: hell, level 60, A capped at T(61) − T(60): one level at most.
    let mut f = fake();
    f.difficulty = 2;
    let (t60, t61, t62) = (100_000_000u32, 110_000_000u32, 125_000_000u32);
    thresholds(&mut f, &[(60, t60), (61, t61), (62, t62)]);
    f.p(P1).stats.insert(12, 60);
    f.p(P1).stats.insert(13, 105_000_000);
    f.p(P1).stats.insert(30, t61 as i32);
    super::experience_reward(&mut f, P1);
    let s = &f.players[&P1].stats;
    // A = 10,000,000 (< 40,000,000): 5,000,000 to level 61, the rest
    // into 61.
    assert_eq!((s[&12], s[&13]), (61, 115_000_000));
    assert_eq!(f.log, ["level up 1"]);
    // At the class's maximum level: only the flags.
    let mut f = fake();
    f.a5_max_level = 25;
    super::experience_reward(&mut f, P1);
    assert_eq!(f.sent_ids(), [0x28]);
    assert!(f.log.is_empty() && !f.players[&P1].stats.contains_key(&13));
}

// Covers: specs/world/quests-act5-2.md §7.1, §7.6
#[test]
fn level_39_in_nightmare_gets_no_reward() {
    // Vector: a level 39 player in nightmare kills the last Ancient → no
    // reward (gate 40).
    let (mut ctl, _) = control();
    let mut f = fake();
    f.difficulty = 1;
    f.p(P1).stats.insert(12, 39);
    start_fight(&mut ctl, &mut f);
    for a in [A540, A541, A542] {
        ctl.monster_killed(&mut f, a, Some(P1));
    }
    let i = rec(&ctl);
    assert!(ctl.records[i].extra.a5.q5.defeated);
    let fl = f.flags(P1);
    assert!(!fl.get(39, 0) && !fl.get(39, 13));
    assert!(f.log.iter().all(|l| !l.starts_with("level up")));
    // The completion flag marks P1 instead.
    assert!(fl.get(39, 14));
    // Level 40 passes.
    assert!(!super::passes_gate(&f, P1));
    f.p(P1).stats.insert(12, 40);
    assert!(super::passes_gate(&f, P1));
}

// Covers: specs/world/quests-act5-2.md §7.4, §7.6, §7.8, §10
#[test]
fn the_fight() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].state = 2;
    f.frame = 100;
    init_objects(&mut ctl, &mut f);
    let x = &ctl.records[i].extra.a5.q5;
    assert_eq!(x.statue_guids, [0x50, 0x51, 0x52]);
    assert_eq!(x.altar_guid, 0x53);
    f.log.clear();
    // 20002: status 3 to all, then the statues are armed (476, 474, 475).
    call(
        &mut ctl,
        &mut f,
        ev(event::SCROLL_MESSAGE, Some(STATUE_NPC_U), 538, 20002),
    );
    assert_eq!(status_msgs(&f, 35), [(P1, 3)]);
    assert_eq!(
        f.log,
        [
            "refresh 82",
            "refresh 80",
            "refresh 81",
            "mode 82 3",
            "free collision 82",
            "event7 82 120",
            "mode 80 3",
            "free collision 80",
            "event7 80 120",
            "mode 81 3",
            "free collision 81",
            "event7 81 120",
        ]
    );
    let x = &ctl.records[i].extra.a5.q5;
    assert!(x.altar_used && x.armed);
    assert_eq!(x.stored_modes, [3, 3, 3]);
    // Stored modes ≠ 0: a second 20002 does not arm again.
    f.log.clear();
    call(
        &mut ctl,
        &mut f,
        ev(event::SCROLL_MESSAGE, Some(STATUE_NPC_U), 538, 20002),
    );
    assert!(f.log.is_empty());
    // Statue 474's event: superunique 45 fails → again at frame + 10.
    super::statue_event(&mut ctl, &mut f, S474);
    assert_eq!(f.log, ["superunique 80 45", "event7 80 110"]);
    // Then the three Ancients.
    f.log.clear();
    for (s, a, class, su) in [
        (S474, A542, 542, 45),
        (S475, A540, 540, 43),
        (S476, A541, 541, 44),
    ] {
        monster(&mut f, a, class, Some(su));
        f.chains.insert(a, QuestChain(vec![35]));
        f.pos.insert(a, (30, 40, RoomId(5)));
        f.a5_superuniques = vec![Some(a)];
        super::statue_event(&mut ctl, &mut f, s);
    }
    let x = &ctl.records[i].extra.a5.q5;
    assert_eq!((x.spawned, x.alive), (3, 3));
    assert!(x.fight_started);
    assert_eq!(x.ancient_guids, [0x62, 0x60, 0x61]);
    assert_eq!(x.ancient_spawned, [true; 3]);
    assert_eq!(x.stored_modes, [4, 4, 4]);
    // A statue in mode 4 does nothing.
    f.log.clear();
    super::statue_event(&mut ctl, &mut f, S474);
    assert!(f.log.is_empty());
    // First kill (540 → statue 475): timer, respawn, missile, FX 18.
    f.sent.clear();
    ctl.monster_killed(&mut f, A540, Some(P1));
    assert_eq!(f.log, ["missile 541 96 -> 81 0x420 1"]);
    assert!(f.sent.iter().any(|m| m.1 == [0x89, 18]));
    let x = &ctl.records[i].extra.a5.q5;
    assert_eq!(x.respawn, [false, true, false]);
    assert_eq!(x.alive, 2);
    assert!(x.timer && !x.defeated);
    assert_eq!(ctl.timers.len(), 1);
    assert_eq!(ctl.timers[0].period, 2);
    // The statue timer brings statue 475 back (mode 1, end-animation
    // event at frame + 0x1000 >> 8).
    f.log.clear();
    for _ in 0..4 {
        ctl.update(&mut f);
    }
    assert_eq!(f.log, ["mode 81 1", "event1 81 116"]);
    assert!(ctl.timers.is_empty());
    let x = &ctl.records[i].extra.a5.q5;
    assert_eq!(x.stored_modes, [4, 2, 4]);
    assert!(!x.timer && x.respawn == [false; 3]);
    // The last two: defeated, P1 rewarded, object 561, chain 36 starts,
    // status 13 to all.
    thresholds(&mut f, &[(25, 100_000), (26, 140_000), (27, 200_000)]);
    f.p(P1).stats.insert(13, 120_000);
    f.p(P1).stats.insert(30, 140_000);
    ctl.monster_killed(&mut f, A541, Some(P1));
    f.log.clear();
    f.sent.clear();
    ctl.monster_killed(&mut f, A542, Some(P1));
    let x = &ctl.records[i].extra.a5.q5;
    assert!(x.defeated && x.alive == 0);
    let fl = f.flags(P1);
    assert!(fl.get(39, 0) && fl.get(39, 13) && !fl.get(39, 14));
    // A = T(26) − T(25) = 40,000: 20,000 to level 26, 20,000 into it.
    let s = &f.players[&P1].stats;
    assert_eq!((s[&12], s[&13]), (26, 160_000));
    assert_eq!(ctl.records[i].state, 5);
    assert_eq!(ctl.records[i].status, 13);
    assert_eq!(
        f.log,
        [
            "missile 541 98 -> 80 0x420 1",
            "level up 1",
            "object at 98 561 1"
        ]
    );
    let q6 = ctl.record(36).unwrap();
    assert_eq!((q6.state, q6.status), (2, 1));
    assert!(q6.extra.a5.q6.by_sequence);
    assert!(status_msgs(&f, 35).contains(&(P1, 13)));
    assert!(status_msgs(&f, 36).contains(&(P1, 1)));
}

// Covers: specs/world/quests-act5-2.md §7.6, §edge-cases-original-bugs r4
#[test]
fn party_members_anywhere_in_act_v_are_rewarded() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // P1 kills from outside the summit (rewarded first); P2 is its party
    // member in Harrogath.
    f.p(P1).level = Some(118);
    f.players.insert(
        P2,
        Player {
            guid: 2,
            act: Some(4),
            level: Some(109),
            ..Player::default()
        },
    );
    f.p(P2).stats.insert(12, 30);
    f.party.insert(P1, vec![P1, P2]);
    start_fight(&mut ctl, &mut f);
    for a in [A540, A541, A542] {
        ctl.monster_killed(&mut f, a, Some(P1));
    }
    for p in [P1, P2] {
        let fl = f.flags(p);
        assert!(fl.get(39, 0) && fl.get(39, 13), "{p:?}");
    }
    // Each reward sends the flags first: 0x28 to both.
    assert!(f.sent.iter().any(|m| m.0 == P2 && m.1[0] == 0x28));
    // An intro record: defeated, nothing else.
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].not_intro = false;
    start_fight(&mut ctl, &mut f);
    for a in [A540, A541, A542] {
        ctl.monster_killed(&mut f, a, Some(P1));
    }
    assert!(ctl.records[i].extra.a5.q5.defeated);
    assert!(!f.flags(P1).get(39, 0) && ctl.records[i].state == 0);
}

// Covers: specs/world/quests-act5-2.md §7.6, §edge-cases-original-bugs r5
#[test]
fn town_portal_resets_the_fight() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    start_fight(&mut ctl, &mut f);
    super::altar_operate(&mut ctl, &mut f, ALTAR_U, P1);
    // One Ancient is gone from the game.
    f.monsters.remove(&A541);
    f.log.clear();
    super::town_portal_opened(&mut ctl, &mut f);
    assert_eq!(super::portal_count(&ctl), 1);
    assert_eq!(
        f.log,
        [
            "remove ancient 98",
            "remove ancient 96",
            "preset 4 542",
            "mode 82 0",
            "mode 80 0",
            "mode 81 0",
            "mode 83 0",
        ]
    );
    let x = &ctl.records[i].extra.a5.q5;
    assert!(!x.armed && !x.altar_used && !x.fight_started);
    assert_eq!((x.spawned, x.alive), (1, 1));
    assert_eq!(x.ancient_spawned, [false; 3]);
    assert_eq!(x.stored_modes, [0; 3]);
    assert_eq!(x.altar_mode, 0);
    // Kills while a portal is open do nothing.
    ctl.monster_killed(&mut f, A540, Some(P1));
    assert!(ctl.timers.is_empty());
    // Closing: back to 0, altar unused → stays disarmed.
    super::town_portal_closed(&mut ctl);
    assert_eq!(super::portal_count(&ctl), 0);
    assert!(!ctl.records[i].extra.a5.q5.armed);
    // With the altar used: closing re-arms.
    ctl.records[i].extra.a5.q5.portals = 1;
    ctl.records[i].extra.a5.q5.altar_used = true;
    super::town_portal_closed(&mut ctl);
    assert!(ctl.records[i].extra.a5.q5.armed);
    // `0x0058CF90` (the Ancients' AI): armed := 0.
    super::disarm(&mut ctl);
    assert!(!ctl.records[i].extra.a5.q5.armed);
    // A spawn after a portal opened: reset instead of the spawn.
    let (mut ctl, _) = control();
    let mut f = fake();
    init_objects(&mut ctl, &mut f);
    ctl.records[i].extra.a5.q5.portals = 1;
    f.log.clear();
    super::statue_event(&mut ctl, &mut f, S475);
    assert!(f.log.iter().all(|l| !l.starts_with("superunique")));
    assert!(f.log.contains(&"mode 81 0".to_string()));
    // Defeated: a portal changes nothing.
    ctl.records[i].extra.a5.q5.portals = 0;
    ctl.records[i].extra.a5.q5.defeated = true;
    super::town_portal_opened(&mut ctl, &mut f);
    assert_eq!(super::portal_count(&ctl), 0);
}

// Covers: specs/world/quests-act5-2.md §7.6, §edge-cases-original-bugs r5
#[test]
fn wiped_summit_resets_the_fight() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    f.players.insert(
        P2,
        Player {
            guid: 2,
            act: Some(4),
            level: Some(120),
            ..Player::default()
        },
    );
    start_fight(&mut ctl, &mut f);
    // P1 dies (mode 0), P2 alive (mode 1): counted 1, nothing reset.
    f.a5_modes.insert(P2, 1);
    f.a5_modes.insert(P1, 0);
    super::player_died(&mut ctl, &mut f, P1);
    assert_eq!(ctl.records[i].extra.a5.q5.living, 1);
    assert!(ctl.records[i].extra.a5.q5.armed);
    // P2 dead too (0x11): reset.
    f.a5_modes.insert(P2, 0x11);
    super::player_died(&mut ctl, &mut f, P2);
    let x = &ctl.records[i].extra.a5.q5;
    assert_eq!(x.living, 0);
    assert!(!x.armed && !x.altar_used && !x.fight_started);
    // Not armed any more: the next death is not counted.
    ctl.records[i].extra.a5.q5.living = 7;
    super::player_died(&mut ctl, &mut f, P1);
    assert_eq!(ctl.records[i].extra.a5.q5.living, 7);
    // Off the summit: not counted either.
    ctl.records[i].extra.a5.q5.armed = true;
    f.p(P1).level = Some(118);
    super::player_died(&mut ctl, &mut f, P1);
    assert_eq!(ctl.records[i].extra.a5.q5.living, 7);
}

// Covers: specs/world/quests-act5-2.md §7.8, §edge-cases-original-bugs r6
#[test]
fn altar() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].state = 1;
    ctl.records[i].flags = 0x21;
    ctl.records[i].extra.a5.q5.portals = 2;
    // P1's town portal (class 59) in level 120 with its partner
    // (`0x0058D2C0` → `0x00535430`, `quests-helpers.md` §7): chain 35's
    // hook runs for each, then each is freed.
    let (tp, partner) = (UnitId(0x90), UnitId(0x91));
    f.h_tp.insert(P1, 0x90);
    f.objects.insert(tp, (0x90, 59, 0));
    f.unit_levels.insert(tp, 120);
    f.h_partner.insert(tp, partner);
    super::altar_init(&mut ctl, &mut f, ALTAR_U);
    f.log.clear();
    assert_eq!(super::altar_operate(&mut ctl, &mut f, ALTAR_U, P1), 0);
    assert_eq!(
        f.log,
        [
            "free portal 144",
            "free portal 145",
            "message 1 83 20002",
            "mode 83 1",
        ]
    );
    assert_eq!(ctl.records[i].extra.a5.q5.portals, 0);
    let r = &ctl.records[i];
    // Edge case 6: status 3 sent with the flags byte kept.
    assert_eq!((r.state, r.status, r.flags), (2, 3, 0x21));
    assert_eq!(f.sent, [(P1, hex("5D 23 21 03 0000"))]);
    assert_eq!(r.extra.a5.q5.altar_mode, 2);
    // Mode ≠ 0: nothing.
    f.log.clear();
    super::altar_operate(&mut ctl, &mut f, ALTAR_U, P1);
    assert!(f.log.is_empty());
    // The init restores the stored mode.
    super::altar_init(&mut ctl, &mut f, ALTAR_U);
    assert_eq!(f.log, ["mode 83 2"]);
    // Not-intro with state ≥ 4: no message, only the mode.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.records[i].state = 4;
    super::altar_operate(&mut ctl, &mut f, ALTAR_U, P1);
    assert_eq!(f.log, ["mode 83 1"]);
    assert!(f.sent.is_empty());
    // Intro: message (no portals → none closed), no state or status.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.records[i].not_intro = false;
    ctl.records[i].state = 4;
    super::altar_operate(&mut ctl, &mut f, ALTAR_U, P1);
    assert_eq!(f.log, ["message 1 83 20002", "mode 83 1"]);
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (4, 0));
}

// Covers: specs/world/quests-act5-2.md §7.8
#[test]
fn statues_doors_and_the_invisible_ancient() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    // Statue inits take the stored mode; operates refuse.
    ctl.records[i].extra.a5.q5.stored_modes = [4, 2, 0];
    init_objects(&mut ctl, &mut f);
    assert_eq!(&f.log[..3], ["mode 80 4", "mode 81 2", "mode 82 0"]);
    f.log.clear();
    assert_eq!(super::statue_operate(&mut f, P1), 0);
    assert_eq!(f.log, ["sound 1 19"]);
    // Door 547 init: closed, or open when done before.
    f.log.clear();
    super::keep_door_init(&ctl, &mut f, DOOR_U);
    ctl.records[i].extra.a5.q5.done_before = true;
    super::keep_door_init(&ctl, &mut f, DOOR_U);
    assert_eq!(f.log, ["mode 84 0", "mode 84 2"]);
    // Operate: lacking 39.0 and 39.1 → refused.
    f.log.clear();
    f.objects.get_mut(&DOOR_U).unwrap().2 = 0;
    super::keep_door_operate(&ctl, &mut f, DOOR_U, P1);
    assert_eq!(f.log, ["sound 1 19"]);
    // 39.0 but not defeated (not-intro) → refused.
    f.p(P1).quests.flags[0].set(39, 0);
    f.log.clear();
    super::keep_door_operate(&ctl, &mut f, DOOR_U, P1);
    assert_eq!(f.log, ["sound 1 19"]);
    // Intro without a fight started: opens (mode 0 → 1).
    ctl.records[i].not_intro = false;
    f.log.clear();
    super::keep_door_operate(&ctl, &mut f, DOOR_U, P1);
    assert_eq!(f.log, ["mode 84 1", "event1 84 16", "refresh 84"]);
    // Defeated, mode 2: the warp.
    ctl.records[i].not_intro = true;
    ctl.records[i].extra.a5.q5.defeated = true;
    f.objects.get_mut(&DOOR_U).unwrap().2 = 2;
    f.pos.insert(DOOR_U, (10, 20, crate::units::RoomId(7)));
    f.log.clear();
    super::keep_door_operate(&ctl, &mut f, DOOR_U, P1);
    assert_eq!(f.log, ["stairs 1 84", "room portal RoomId(7) true"]);
    // Object 561: 39.0 and not 39.4 → message 20169; 20169 sets 39.4.
    f.log.clear();
    super::invisible_ancient_operate(&mut f, UnitId(0x70), P1);
    assert_eq!(f.log, ["message 1 112 20169"]);
    call(&mut ctl, &mut f, ev(event::SCROLL_MESSAGE, None, 0, 20169));
    assert!(f.flags(P1).get(39, 4));
    f.log.clear();
    super::invisible_ancient_operate(&mut f, UnitId(0x70), P1);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act5-2.md §7.8, §7.5
#[test]
fn summit_door() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    // Init: seen, GUID, mode := +0x60 (0); not defeated → +0x60 := 2,
    // mode 1.
    super::summit_door_init(&mut ctl, &mut f, SUMMIT_DOOR_U);
    let x = &ctl.records[i].extra.a5.q5;
    assert!(x.door_seen && x.door_guid == 0x55 && x.door_mode == 2);
    assert_eq!(f.log, ["mode 85 0", "mode 85 1"]);
    // Operate, not defeated: 1 → sound, 2; 2 → two sounds, 2; 0 → 1.
    f.log.clear();
    super::summit_door_operate(&mut ctl, &mut f, SUMMIT_DOOR_U, P1);
    assert_eq!(f.log, ["sound 1 19", "mode 85 2"]);
    f.log.clear();
    super::summit_door_operate(&mut ctl, &mut f, SUMMIT_DOOR_U, P1);
    assert_eq!(f.log, ["sound 1 19", "sound 1 19", "mode 85 2"]);
    f.objects.get_mut(&SUMMIT_DOOR_U).unwrap().2 = 0;
    ctl.records[i].extra.a5.q5.door_mode = 0;
    f.log.clear();
    super::summit_door_operate(&mut ctl, &mut f, SUMMIT_DOOR_U, P1);
    assert_eq!(f.log, ["mode 85 1"]);
    assert_eq!(ctl.records[i].extra.a5.q5.door_mode, 2);
    // Defeated: 1 → 0; 2 (fight started) → 0 twice; 0 → the warp
    // `0x0059D9D0` (`0x0058D758`).
    ctl.records[i].extra.a5.q5.defeated = true;
    f.log.clear();
    super::summit_door_operate(&mut ctl, &mut f, SUMMIT_DOOR_U, P1);
    assert_eq!(f.log, ["mode 85 0"]);
    assert_eq!(ctl.records[i].extra.a5.q5.door_mode, 0);
    f.objects.get_mut(&SUMMIT_DOOR_U).unwrap().2 = 2;
    ctl.records[i].extra.a5.q5.fight_started = true;
    f.log.clear();
    super::summit_door_operate(&mut ctl, &mut f, SUMMIT_DOOR_U, P1);
    assert_eq!(f.log, ["mode 85 0", "mode 85 0"]);
    f.log.clear();
    super::summit_door_operate(&mut ctl, &mut f, SUMMIT_DOOR_U, P1);
    assert_eq!(f.log, ["stairs 1 85"]);
    // Entering the summit closes a seen door with +0x60 = 0 (mode 1).
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.records[i].state = 3;
    let x = &mut ctl.records[i].extra.a5.q5;
    x.door_seen = true;
    x.door_guid = 0x55;
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 118, 120),
    );
    assert_eq!(f.log, ["mode 85 1"]);
    assert_eq!(ctl.records[i].extra.a5.q5.door_mode, 2);
}

// Covers: specs/world/quests-act5-2.md §7.8, §10
#[test]
fn summit_warp_check() {
    let (mut ctl, _) = control();
    assert_eq!(warp_check(120, 118), WarpCheck::Delegate(0x0058_D090));
    assert!(!super::summit_warp_open(&ctl));
    let i = rec(&ctl);
    // The altar plays no part (`0x0058D090` reads not-intro and +0x00
    // only; REC-246): unused or used, the exits stay closed.
    assert!(!super::altar_used(&ctl));
    ctl.records[i].extra.a5.q5.altar_mode = 2;
    ctl.records[i].extra.a5.q5.fight_started = true;
    assert!(super::altar_used(&ctl));
    assert!(!super::summit_warp_open(&ctl));
    ctl.records[i].extra.a5.q5.defeated = true;
    assert!(super::summit_warp_open(&ctl));
    ctl.records[i].extra.a5.q5.defeated = false;
    ctl.records[i].not_intro = false;
    assert!(super::summit_warp_open(&ctl));
}

// Covers: specs/world/quests-act5-2.md §7.2, §7.3, §7.4
#[test]
fn qual_kehk_starts_the_quest() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].state = 1;
    // Qual-Kehk in state 1: table state 0 (20153); he wants to talk.
    assert_eq!(text(&mut ctl, &mut f, QUAL_U), [20153]);
    assert!(super::active(&ctl, &mut f, i, P1, 515, 0));
    assert!(!super::active(&ctl, &mut f, i, P1, 511, 0));
    // The statue NPC before the altar: table state 4 (20002).
    assert_eq!(text(&mut ctl, &mut f, STATUE_NPC_U), [20002]);
    // 20153 from someone else: nothing.
    call(
        &mut ctl,
        &mut f,
        ev(event::SCROLL_MESSAGE, Some(LARZUK_U), 511, 20153),
    );
    assert_eq!(ctl.records[i].state, 1);
    call(
        &mut ctl,
        &mut f,
        ev(event::SCROLL_MESSAGE, Some(QUAL_U), 515, 20153),
    );
    let r = &ctl.records[i];
    assert_eq!((r.state, r.status), (2, 1));
    assert!(r.extra.a5.q5.qual_started);
    assert!(f.flags(P1).get(39, 2));
    assert_eq!(status_msgs(&f, 35), [(P1, 1)]);
    // A player under the gate is not iterated.
    let (mut ctl, _) = control();
    let mut f = fake();
    f.p(P1).stats.insert(12, 19);
    ctl.records[i].state = 1;
    call(
        &mut ctl,
        &mut f,
        ev(event::SCROLL_MESSAGE, Some(QUAL_U), 515, 20153),
    );
    assert!(!f.flags(P1).get(39, 2));
    // Chat end with Qual-Kehk: status 1 to all, +0x01 := 0 (once).
    f.sent.clear();
    ctl.records[i].status = 0;
    call(
        &mut ctl,
        &mut f,
        ev(event::NPC_DEACTIVATE, Some(QUAL_U), 0, 0),
    );
    assert_eq!(ctl.records[i].status, 1);
    assert!(!ctl.records[i].extra.a5.q5.qual_started);
    assert_eq!(f.sent.len(), 1);
    call(
        &mut ctl,
        &mut f,
        ev(event::NPC_DEACTIVATE, Some(QUAL_U), 0, 0),
    );
    assert_eq!(f.sent.len(), 1);
    // Intro: no lines.
    ctl.records[i].not_intro = false;
    assert!(text(&mut ctl, &mut f, QUAL_U).is_empty());
}

// Covers: specs/world/quests-act5-2.md §7.3, §7.4
#[test]
fn post_quest_lines() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].state = 5;
    f.p(P1).quests.flags[0].set(39, 0);
    // 39.5 clear → table state 5 (menu 0); set with 39.13 → 3 (menu 2).
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [20167]);
    call(
        &mut ctl,
        &mut f,
        ev(event::SCROLL_MESSAGE, Some(LARZUK_U), 511, 20167),
    );
    assert!(f.flags(P1).get(39, 5));
    assert!(text(&mut ctl, &mut f, LARZUK_U).is_empty());
    f.p(P1).quests.flags[0].set(39, 13);
    assert_eq!(text(&mut ctl, &mut f, LARZUK_U), [20167]);
    // Statue NPCs and others say nothing after the quest.
    assert!(text(&mut ctl, &mut f, STATUE_NPC_U).is_empty());
    // Each line sets its own bit, only from its NPC.
    for (msg, npc, b) in [
        (20165, 520, 6),
        (20166, 512, 7),
        (20168, 513, 8),
        (20164, 515, 9),
    ] {
        call(&mut ctl, &mut f, ev(event::SCROLL_MESSAGE, None, 511, msg));
        assert!(!f.flags(P1).get(39, b));
        call(&mut ctl, &mut f, ev(event::SCROLL_MESSAGE, None, npc, msg));
        assert!(f.flags(P1).get(39, b), "{msg}");
    }
}

// Covers: specs/world/quests-act5-2.md §7.5
#[test]
fn level_changes_game_start_and_leave() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    ctl.records[i].state = 2;
    ctl.records[i].guids.add(1);
    // Leaving Harrogath in state 2: quick remove, state 3, status 1, 39.3.
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 109, 110),
    );
    let r = &ctl.records[i];
    assert!(r.guids.0.is_empty());
    assert_eq!((r.state, r.status), (3, 1));
    assert!(f.flags(P1).get(39, 3));
    // Entering the summit with state 3, status 1: status 2 to all.
    f.sent.clear();
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 118, 120),
    );
    assert_eq!(ctl.records[i].status, 2);
    assert_eq!(status_msgs(&f, 35), [(P1, 2)]);
    // State < 3: state 3 and the iterate.
    let (mut ctl, _) = control();
    let mut f = fake();
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 118, 120),
    );
    assert_eq!((ctl.records[i].state, ctl.records[i].status), (3, 0));
    // Intro: leaving town stops after the quick remove.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.records[i].not_intro = false;
    ctl.records[i].state = 2;
    call(
        &mut ctl,
        &mut f,
        ev(event::CHANGED_LEVEL, Some(P1), 109, 110),
    );
    assert_eq!(ctl.records[i].state, 2);
    // Game start: 39.0 → +0x4A; 39.3 → status 1, state 3; 39.2 → 2.
    for (b, want) in [
        (0, (0, 0, true)),
        (15, (0, 0, true)),
        (3, (3, 1, false)),
        (2, (2, 1, false)),
    ] {
        let (mut ctl, _) = control();
        let mut f = fake();
        f.p(P1).quests.flags[0].set(39, b);
        call(
            &mut ctl,
            &mut f,
            ev(event::PLAYER_STARTED_GAME, Some(P1), 0, 0),
        );
        let r = &ctl.records[i];
        assert_eq!(
            (r.state, r.status, r.extra.a5.q5.done_before),
            want,
            "bit {b}"
        );
        assert!(f.sent.is_empty());
    }
    // Leave.
    let (mut ctl, _) = control();
    let mut f = fake();
    ctl.records[i].guids.add(1);
    assert!(call(
        &mut ctl,
        &mut f,
        ev(event::PLAYER_LEAVES_GAME, Some(P1), 0, 0)
    ));
    assert!(ctl.records[i].guids.0.is_empty());
}

// Covers: specs/world/quests-act5-2.md §7.9
#[test]
fn ancients_ai_hooks() {
    let (mut ctl, _) = control();
    let i = rec(&ctl);
    assert_eq!(super::portal_count(&ctl), 0);
    ctl.records[i].extra.a5.q5.portals = 2;
    assert_eq!(super::portal_count(&ctl), 2);
    ctl.records[i].extra.a5.q5.armed = true;
    super::disarm(&mut ctl);
    assert!(!ctl.records[i].extra.a5.q5.armed);
}

// Covers: specs/world/quests-act5-2.md §7.10
#[test]
fn sequence_function() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let i = rec(&ctl);
    assert!(super::super::sequence(&mut ctl, &mut f, 35));
    assert_eq!(ctl.records[i].state, 1);
    // State 3: holds.
    ctl.records[i].state = 3;
    assert!(super::super::sequence(&mut ctl, &mut f, 35));
    assert_eq!(ctl.records[i].state, 3);
    // State 5: seq(36).
    ctl.records[i].state = 5;
    assert!(super::super::sequence(&mut ctl, &mut f, 35));
    assert_eq!(ctl.record(36).unwrap().state, 2);
}
