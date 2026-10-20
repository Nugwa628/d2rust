// Spec: specs/world/quests-act3.md §9 (Act III gossip and intro records)
//! A3Q0 Hratli (chain 14), A3Q7 the Dark Wanderer (chain 28) and the
//! Act III intro (chain 39).

use super::*;
use act3::Timer;

const SPAWNED: UnitId = UnitId(0x50);
const WANDERER: UnitId = UnitId(0x60);
const R1: RoomId = RoomId(1);
/// The dummies' / the wanderer object's init records
/// (`quests-act3-2.md` §11.7).
const START_AT: act3::InitPoint = act3::InitPoint {
    room: R1,
    x: 100,
    y: 200,
};
const END_AT: act3::InitPoint = act3::InitPoint {
    room: R1,
    x: 300,
    y: 400,
};

fn scroll(p: UnitId, class: u16, msg: u32) -> EventArgs {
    EventArgs {
        event: event::SCROLL_MESSAGE,
        player: Some(p),
        a: u32::from(class),
        b: msg,
        ..EventArgs::default()
    }
}

fn started(p: UnitId) -> EventArgs {
    EventArgs {
        event: event::PLAYER_STARTED_GAME,
        player: Some(p),
        target: Some(p),
        ..EventArgs::default()
    }
}

fn active(ctl: &QuestControl, f: &mut Fake3, chain: u8, p: UnitId, class: u16) -> bool {
    let i = ctl.find(chain).unwrap();
    act3::active(ctl, f, i, p, class)
}

fn monster(f: &mut Fake3, u: UnitId, class: u16) {
    f.f.monsters.insert(
        u,
        (
            u.0,
            class,
            UnitKind::Monster {
                class: u32::from(class),
                superunique: None,
                owner: None,
            },
        ),
    );
}

fn unhandled(f: &Fake3) -> Vec<String> {
    f.log()
        .into_iter()
        .filter(|l| l.starts_with("unhandled"))
        .collect()
}

// ------------------------------------------------------------ §9.1

// Covers: specs/world/quests-act3.md §9.1
#[test]
fn hratli_text_and_active() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    // Not a sorceress: table state 0 (465); a sorceress: state 1 (466).
    assert_eq!(text(&mut ctl, &mut f, 14, P1, HRATLI_U), [(465, 0)]);
    assert!(active(&ctl, &mut f, 14, P1, act3::npc::HRATLI));
    f.p(P1).class = 1;
    assert_eq!(text(&mut ctl, &mut f, 14, P1, HRATLI_U), [(466, 0)]);
    // Other NPCs: nothing.
    assert!(text(&mut ctl, &mut f, 14, P1, ALKOR_U).is_empty());
    assert!(!active(&ctl, &mut f, 14, P1, act3::npc::ALKOR));
    // 16.0 set: nothing.
    f.p(P1).quests.flags[0].set(16, bit::REWARD_GRANTED);
    assert!(text(&mut ctl, &mut f, 14, P1, HRATLI_U).is_empty());
    assert!(!active(&ctl, &mut f, 14, P1, act3::npc::HRATLI));
    // The status function returns false.
    let i = ctl.find(14).unwrap();
    let pf = f.flags(P1);
    assert_eq!(act1::status_fn(&ctl, &mut f, i, P1, &pf, 0x005B_6F90), None);
    assert!(unhandled(&f).is_empty());
}

// Covers: specs/world/quests-act3.md §9.1
#[test]
fn hratli_messages_and_start() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    // Other messages or NPCs: nothing.
    call(&mut ctl, &mut f, 14, scroll(P1, act3::npc::HRATLI, 467));
    call(&mut ctl, &mut f, 14, scroll(P1, act3::npc::ALKOR, 465));
    assert!(!f.flags(P1).get(16, bit::REWARD_GRANTED));
    assert!(!ctl.game.get(16, bit::PRIMARY_GOAL_DONE));
    for msg in [465, 466] {
        let (mut ctl, _) = control();
        let mut f = Fake3::new();
        call(&mut ctl, &mut f, 14, scroll(P1, act3::npc::HRATLI, msg));
        assert!(f.flags(P1).get(16, bit::REWARD_GRANTED));
        assert!(ctl.game.get(16, bit::PRIMARY_GOAL_DONE));
    }
    // Through C→S 0x31 to Hratli.
    say(&mut ctl, &mut f, P1, HRATLI_U, 466);
    assert!(f.flags(P1).get(16, bit::REWARD_GRANTED));
    assert!(ctl.game.get(16, bit::PRIMARY_GOAL_DONE));

    // Event 13: 16.0 → game 16.13.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    call(&mut ctl, &mut f, 14, started(P1));
    assert!(!ctl.game.get(16, bit::PRIMARY_GOAL_DONE));
    f.p(P1).quests.flags[0].set(16, bit::REWARD_GRANTED);
    call(&mut ctl, &mut f, 14, started(P1));
    assert!(ctl.game.get(16, bit::PRIMARY_GOAL_DONE));
    assert!(unhandled(&f).is_empty());
}

// Covers: specs/world/quests-act3.md §9.1
#[test]
fn hratli_start_dummy() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    // Spawn fails: nothing stored.
    act3::hratli_start_init(&mut ctl, &mut f, START_AT);
    assert_eq!(
        ctl.record(14).unwrap().extra.act3.q0,
        act3::q0::Extra::default()
    );
    // Spawned: +0x0C := GUID, +0x00 := 1.
    f.f.spawns = vec![Some(SPAWNED)];
    act3::hratli_start_init(&mut ctl, &mut f, START_AT);
    let x = &ctl.record(14).unwrap().extra.act3.q0;
    assert!(x.start_spawned && x.hratli_guid == SPAWNED.0);
    assert_eq!(
        f.log(),
        [
            "spawn 253 100 200 mode 1 r 4294967295",
            "spawn 253 100 200 mode 1 r 4294967295"
        ]
    );
    // +0x00 and Hratli exists: no spawn.
    monster(&mut f, SPAWNED, act3::npc::HRATLI);
    f.f.log.clear();
    act3::hratli_start_init(&mut ctl, &mut f, START_AT);
    assert!(f.log().is_empty());
    // Hratli gone: spawned again.
    f.f.monsters.remove(&SPAWNED);
    f.f.spawns = vec![Some(UnitId(0x51))];
    act3::hratli_start_init(&mut ctl, &mut f, START_AT);
    assert_eq!(ctl.record(14).unwrap().extra.act3.q0.hratli_guid, 0x51);
    // Game 16.13: nothing.
    ctl.game.set(16, bit::PRIMARY_GOAL_DONE);
    f.f.log.clear();
    f.f.spawns = vec![Some(UnitId(0x52))];
    act3::hratli_start_init(&mut ctl, &mut f, START_AT);
    assert!(f.log().is_empty());
    assert_eq!(ctl.record(14).unwrap().extra.act3.q0.hratli_guid, 0x51);
}

// Covers: specs/world/quests-act3.md §9.1
#[test]
fn hratli_end_dummy() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    // Game 16.13 clear: only +0x02 and the position.
    f.f.spawns = vec![Some(SPAWNED)];
    act3::hratli_end_init(&mut ctl, &mut f, END_AT);
    let x = ctl.record(14).unwrap().extra.act3.q0.clone();
    assert!(x.end_seen && (x.end_x, x.end_y) == (300, 400));
    assert!(!x.end_spawned && x.hratli_guid == 0);
    assert!(f.log().is_empty());

    // The start Hratli exists: +0x03 := 1, no spawn.
    ctl.game.set(16, bit::PRIMARY_GOAL_DONE);
    {
        let x = &mut ctl.record_mut(14).unwrap().extra.act3.q0;
        x.start_spawned = true;
        x.hratli_guid = HRATLI_U.0;
    }
    act3::hratli_end_init(&mut ctl, &mut f, END_AT);
    let x = ctl.record(14).unwrap().extra.act3.q0.clone();
    assert!(x.start_present && !x.end_spawned);
    assert!(f.log().is_empty());

    // Otherwise spawn there: GUID, +0x01, unit flags 0x3000000; no map
    // AI stored in 1.14d, so none applied.
    ctl.record_mut(14).unwrap().extra.act3.q0.start_spawned = false;
    act3::hratli_end_init(&mut ctl, &mut f, END_AT);
    let x = ctl.record(14).unwrap().extra.act3.q0.clone();
    assert!(x.end_spawned && x.hratli_guid == SPAWNED.0 && !x.ai_applied);
    assert_eq!(
        f.log(),
        [
            "spawn 253 300 400 mode 1 r 4294967295",
            "flags 80 0x3000000"
        ]
    );
    // +0x01 set: nothing more.
    f.f.log.clear();
    f.f.spawns = vec![Some(UnitId(0x51))];
    act3::hratli_end_init(&mut ctl, &mut f, END_AT);
    assert!(f.log().is_empty());

    // A failed spawn stores nothing.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    ctl.game.set(16, bit::PRIMARY_GOAL_DONE);
    act3::hratli_end_init(&mut ctl, &mut f, END_AT);
    let x = ctl.record(14).unwrap().extra.act3.q0.clone();
    assert!(!x.end_spawned && x.hratli_guid == 0 && x.end_seen);
    assert_eq!(f.log(), ["spawn 253 300 400 mode 1 r 4294967295"]);
}

// ------------------------------------------------------------ §9.2

// Covers: specs/world/quests-act3.md §9.2
#[test]
fn wanderer_start_and_no_other_callbacks() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    assert!(ctl.record(28).unwrap().extra.act3.q7.to_spawn);
    call(&mut ctl, &mut f, 28, started(P1));
    assert!(!ctl.game.get(32, bit::PRIMARY_GOAL_DONE));
    assert!(ctl.record(28).unwrap().extra.act3.q7.to_spawn);
    f.p(P1).quests.flags[0].set(32, bit::REWARD_GRANTED);
    call(&mut ctl, &mut f, 28, started(P1));
    assert!(ctl.game.get(32, bit::PRIMARY_GOAL_DONE));
    assert!(!ctl.record(28).unwrap().extra.act3.q7.to_spawn);
    assert!(unhandled(&f).is_empty());
    // Status and active functions return false.
    let i = ctl.find(28).unwrap();
    let pf = f.flags(P1);
    assert_eq!(act1::status_fn(&ctl, &mut f, i, P1, &pf, 0x005B_D0B0), None);
    assert!(!act1::active_fn(
        &ctl,
        &mut f,
        i,
        P1,
        act3::npc::HRATLI,
        0x005B_D0C0
    ));
    assert!(unhandled(&f).is_empty());
}

// Covers: specs/world/quests-act3.md §9.2
#[test]
fn wanderer_object_init() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.rooms.insert(R1, (0, 0, 1000, 1000));
    // Spawn fails: +0x01 stays, +0x00 := 1.
    act3::wanderer_init(&mut ctl, &mut f, START_AT);
    let x = ctl.record(28).unwrap().extra.act3.q7.clone();
    assert!(x.to_spawn && x.seen && (x.target_x, x.target_y) == (107, 200));
    // Spawned at (x + 7, y): +0x01 := 0.
    f.f.spawns = vec![Some(WANDERER)];
    act3::wanderer_init(&mut ctl, &mut f, START_AT);
    assert!(!ctl.record(28).unwrap().extra.act3.q7.to_spawn);
    // +0x01 clear: no spawn.
    act3::wanderer_init(&mut ctl, &mut f, START_AT);
    assert_eq!(
        f.log(),
        [
            "spawn 368 107 200 mode 1 r 4294967295",
            "spawn 368 107 200 mode 1 r 4294967295"
        ]
    );
    // Without +0x01 from the start: only +0x00.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    ctl.record_mut(28).unwrap().extra.act3.q7.to_spawn = false;
    act3::wanderer_init(&mut ctl, &mut f, START_AT);
    let x = ctl.record(28).unwrap().extra.act3.q7.clone();
    assert!(x.seen && (x.target_x, x.target_y) == (0, 0));
    assert!(f.log().is_empty());
}

// Covers: specs/world/quests-act3.md §9.2
#[test]
fn wanderer_walk_target() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    monster(&mut f, WANDERER, act3::npc::DARK_WANDERER);
    f.f.pos.insert(WANDERER, (107, 200, R1));
    // Without +0x00: none.
    assert_eq!(act3::wanderer_target(&mut ctl, &mut f, WANDERER), None);
    {
        let x = &mut ctl.record_mut(28).unwrap().extra.act3.q7;
        x.seen = true;
        (x.target_x, x.target_y) = (107, 200);
    }
    // (X, Y − 20) blocked → (X, Y − 11).
    f.blocked.insert((107, 180));
    assert_eq!(
        act3::wanderer_target(&mut ctl, &mut f, WANDERER),
        Some((107, 189))
    );
    assert_eq!(
        f.log(),
        ["blocked? 107 180 0x3c01", "blocked? 107 189 0x3c01"]
    );
    let x = ctl.record(28).unwrap().extra.act3.q7.clone();
    assert!(x.target_fixed && (x.target_x, x.target_y) == (107, 189));
    // Later calls: the stored target, no test.
    f.f.log.clear();
    f.blocked.clear();
    assert_eq!(
        act3::wanderer_target(&mut ctl, &mut f, WANDERER),
        Some((107, 189))
    );
    assert!(f.log().is_empty());

    // Every candidate blocked → (X, Y − 3), in the order of the tries.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.pos.insert(WANDERER, (107, 200, R1));
    {
        let x = &mut ctl.record_mut(28).unwrap().extra.act3.q7;
        x.seen = true;
        (x.target_x, x.target_y) = (107, 200);
    }
    for c in [(107, 180), (107, 189), (109, 189), (105, 189), (107, 192)] {
        f.blocked.insert(c);
    }
    assert_eq!(
        act3::wanderer_target(&mut ctl, &mut f, WANDERER),
        Some((107, 197))
    );
    assert_eq!(
        f.log(),
        [
            "blocked? 107 180 0x3c01",
            "blocked? 107 189 0x3c01",
            "blocked? 109 189 0x3c01",
            "blocked? 105 189 0x3c01",
            "blocked? 107 192 0x3c01"
        ]
    );
}

// Covers: specs/world/quests-act3.md §9.2
#[test]
fn wanderer_minion_hook() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    monster(&mut f, WANDERER, act3::npc::DARK_WANDERER);
    act3::wanderer_minions(&mut ctl, &mut f, WANDERER);
    let x = ctl.record(28).unwrap().extra.act3.q7.clone();
    assert!(x.timer && x.wanderer_guid == WANDERER.0);
    assert_eq!(ctl.timers.len(), 1);
    let t = ctl.timers[0];
    assert_eq!(
        (t.func, t.chain, t.period),
        (TimerFn::Act3(Timer::WandererMinions), 28, 2)
    );
    // +0x0D set: no second timer.
    act3::wanderer_minions(&mut ctl, &mut f, WANDERER);
    assert_eq!(ctl.timers.len(), 1);
    // +0x0C set: nothing.
    let (mut ctl, _) = control();
    ctl.record_mut(28).unwrap().extra.act3.q7.minions = true;
    act3::wanderer_minions(&mut ctl, &mut f, WANDERER);
    assert!(ctl.timers.is_empty());
    assert!(!ctl.record(28).unwrap().extra.act3.q7.timer);
}

fn minion_setup(lo: u32) -> (QuestControl, Fake3) {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    monster(&mut f, WANDERER, act3::npc::DARK_WANDERER);
    f.f.pos.insert(WANDERER, (50, 60, R1));
    f.f.spot = Some((0, 0));
    ctl.seed = Seed::new(lo, 666);
    {
        let x = &mut ctl.record_mut(28).unwrap().extra.act3.q7;
        x.timer = true;
        x.wanderer_guid = WANDERER.0;
    }
    (ctl, f)
}

fn dummies(f: &Fake3) -> Vec<(i32, i32)> {
    f.log()
        .iter()
        .filter_map(|l| {
            let r = l.strip_prefix("spawn object 131 ")?;
            let v: Vec<i32> = r.split(' ').take(2).map(|s| s.parse().unwrap()).collect();
            Some((v[0] - 50, v[1] - 60))
        })
        .collect()
}

// Covers: specs/world/quests-act3.md §9.2, §edge-cases-original-bugs r15
#[test]
fn minion_timer_vector() {
    // Vector: quest seed {12345, 666} → lo' odd → start index 1 → 7
    // dummies at offsets 1–7.
    let (mut ctl, mut f) = minion_setup(12345);
    f.add_player(P2, 76);
    f.p(P2).act = Some(0);
    f.add_player(UnitId(3), 76);
    f.p(UnitId(3)).quests.flags[0].set(32, bit::REWARD_GRANTED);
    assert!(act3::run_timer(
        &mut ctl,
        &mut f,
        Timer::WandererMinions,
        28
    ));
    let mut s = Seed::new(12345, 666);
    let lo = s.step();
    assert_eq!(lo, 22_752_887);
    assert_eq!(lo & 1, 1);
    assert_eq!(ctl.seed, s);
    // Offsets 1–7 of `0x00741538`: (−3, 3) twice and no (3, 3).
    assert_eq!(
        dummies(&f),
        [(-3, 0), (-3, 3), (0, -3), (0, 3), (3, -3), (3, 0), (-3, 3)]
    );
    assert!(!dummies(&f).contains(&(3, 3)));
    assert!(f
        .log()
        .contains(&"spot at 47 60 3 0x3f11 11 100".to_string()));
    assert!(f
        .log()
        .contains(&"spawn object 131 47 63 room 1".to_string()));
    // Players without 32.0 in Act III get 32.0; nothing is sent.
    assert!(f.flags(P1).get(32, bit::REWARD_GRANTED));
    assert!(!f.flags(P2).get(32, bit::REWARD_GRANTED));
    assert!(f.flags(UnitId(3)).get(32, bit::REWARD_GRANTED));
    assert!(f.f.sent.is_empty());
    let x = ctl.record(28).unwrap().extra.act3.q7.clone();
    assert!(x.minions && !x.timer);
}

// Covers: specs/world/quests-act3.md §9.2
#[test]
fn minion_timer_even_and_misses() {
    // An even lo' starts at index 0: 8 dummies.
    let lo0 = (1..)
        .find(|&lo| Seed::new(lo, 666).step() & 1 == 0)
        .unwrap();
    let (mut ctl, mut f) = minion_setup(lo0);
    assert!(act3::run_timer(
        &mut ctl,
        &mut f,
        Timer::WandererMinions,
        28
    ));
    assert_eq!(dummies(&f), act3::q7::MINION_OFFSETS);
    // No free spot: no dummy (the seed still steps).
    let (mut ctl, mut f) = minion_setup(12345);
    f.f.spot = None;
    assert!(act3::run_timer(
        &mut ctl,
        &mut f,
        Timer::WandererMinions,
        28
    ));
    assert!(dummies(&f).is_empty());
    assert_eq!(f.log().len(), 7);
    assert_ne!(ctl.seed, Seed::new(12345, 666));
    // The wanderer gone: no draw, no dummy; flags still set.
    let (mut ctl, mut f) = minion_setup(12345);
    f.f.monsters.remove(&WANDERER);
    assert!(act3::run_timer(
        &mut ctl,
        &mut f,
        Timer::WandererMinions,
        28
    ));
    assert_eq!(ctl.seed, Seed::new(12345, 666));
    assert!(f.log().is_empty());
    assert!(f.flags(P1).get(32, bit::REWARD_GRANTED));
    assert!(ctl.record(28).unwrap().extra.act3.q7.minions);
    // +0x0C already set: nothing at all.
    let (mut ctl, mut f) = minion_setup(12345);
    ctl.record_mut(28).unwrap().extra.act3.q7.minions = true;
    assert!(act3::run_timer(
        &mut ctl,
        &mut f,
        Timer::WandererMinions,
        28
    ));
    assert_eq!(ctl.seed, Seed::new(12345, 666));
    assert!(f.log().is_empty());
    assert!(!f.flags(P1).get(32, bit::REWARD_GRANTED));
}

// Covers: specs/world/quests-act3.md §9.2
#[test]
fn minion_timer_through_updater() {
    let (mut ctl, mut f) = minion_setup(12345);
    {
        let x = &mut ctl.record_mut(28).unwrap().extra.act3.q7;
        x.timer = false;
    }
    act3::wanderer_minions(&mut ctl, &mut f, WANDERER);
    // Period 2: due at tick 2, runs when due < tick (the third update).
    ctl.update(&mut f);
    ctl.update(&mut f);
    assert!(dummies(&f).is_empty());
    ctl.update(&mut f);
    assert_eq!(dummies(&f).len(), 7);
    assert!(ctl.timers.is_empty());
}

// ------------------------------------------------------------ §9.3

// Covers: specs/world/quests-act3.md §9.3
#[test]
fn intro_text() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    // Amazon (class 0): Asheara's special line; others the plain one.
    assert_eq!(text(&mut ctl, &mut f, 39, P1, ASHEARA_U), [(491, 0)]);
    assert_eq!(text(&mut ctl, &mut f, 39, P1, ALKOR_U), [(501, 0)]);
    assert_eq!(text(&mut ctl, &mut f, 39, P1, ORMUS_U), [(514, 0)]);
    assert_eq!(text(&mut ctl, &mut f, 39, P1, MESHIF2_U), [(478, 0)]);
    assert_eq!(text(&mut ctl, &mut f, 39, P1, CAIN3_U), [(458, 0)]);
    assert_eq!(text(&mut ctl, &mut f, 39, P1, NATALYA_U), [(453, 0)]);
    // Hratli: nothing.
    assert!(text(&mut ctl, &mut f, 39, P1, HRATLI_U).is_empty());
    // The matching classes: necromancer 2, paladin 3, barbarian 4.
    for (class, n, want) in [
        (2, ALKOR_U, 502),
        (3, ORMUS_U, 515),
        (4, MESHIF2_U, 479),
        (4, ASHEARA_U, 490),
    ] {
        f.p(P1).class = class;
        assert_eq!(text(&mut ctl, &mut f, 39, P1, n), [(want, 0)]);
    }
    // Class 7 (none) never matches: Natalya and Cain stay on state 0.
    for class in 0..7 {
        f.p(P1).class = class;
        assert_eq!(text(&mut ctl, &mut f, 39, P1, NATALYA_U), [(453, 0)]);
        assert_eq!(text(&mut ctl, &mut f, 39, P1, CAIN3_U), [(458, 0)]);
    }
    assert!(unhandled(&f).is_empty());
}

// Covers: specs/world/quests-act3.md §9.3
#[test]
fn intro_bits() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let cases: [(u16, UnitId, &[u32]); 6] = [
        (act3::npc::CAIN3, CAIN3_U, &[458]),
        (act3::npc::ASHEARA, ASHEARA_U, &[490, 491]),
        (act3::npc::ALKOR, ALKOR_U, &[501, 502]),
        (act3::npc::ORMUS, ORMUS_U, &[514, 515]),
        (act3::npc::MESHIF2, MESHIF2_U, &[478, 479]),
        (act3::npc::NATALYA, NATALYA_U, &[453]),
    ];
    for (class, n, msgs) in cases {
        for &m in msgs {
            let (mut ctl, _) = control();
            let mut f = Fake3::new();
            // Another NPC's message: no bit.
            call(&mut ctl, &mut f, 39, scroll(P1, act3::npc::HRATLI, m));
            assert_eq!(f.p(P1).quests.first_talk[0], [0; 8]);
            call(&mut ctl, &mut f, 39, scroll(P1, class, m));
            assert_eq!(f.p(P1).quests.first_talk[0], first_talk_bits(class));
            assert!(text(&mut ctl, &mut f, 39, P1, n).is_empty());
        }
    }
    // A message of another NPC's set: nothing.
    call(&mut ctl, &mut f, 39, scroll(P1, act3::npc::ASHEARA, 501));
    assert_eq!(f.p(P1).quests.first_talk[0], [0; 8]);
    // Through C→S 0x31; the bit is per difficulty.
    f.f.difficulty = 1;
    say(&mut ctl, &mut f, P1, ORMUS_U, 514);
    assert_eq!(f.p(P1).quests.first_talk[0], [0; 8]);
    assert_eq!(
        f.p(P1).quests.first_talk[1],
        first_talk_bits(act3::npc::ORMUS)
    );
}

// Covers: specs/world/quests-act3.md §9.3
#[test]
fn intro_active_and_status() {
    let (ctl, _) = control();
    let mut f = Fake3::new();
    assert!(active(&ctl, &mut f, 39, P1, act3::npc::CAIN3));
    for c in [
        act3::npc::ASHEARA,
        act3::npc::ALKOR,
        act3::npc::ORMUS,
        act3::npc::MESHIF2,
        act3::npc::NATALYA,
        act3::npc::HRATLI,
    ] {
        assert!(!active(&ctl, &mut f, 39, P1, c));
    }
    f.p(P1).quests.hear(0, act3::npc::CAIN3);
    assert!(!active(&ctl, &mut f, 39, P1, act3::npc::CAIN3));
    let i = ctl.find(39).unwrap();
    let pf = f.flags(P1);
    assert_eq!(act1::status_fn(&ctl, &mut f, i, P1, &pf, 0x005B_6E20), None);
    assert!(unhandled(&f).is_empty());
}

/// Field A of one difficulty with only NPC `class`'s first-talk bit
/// (`formats/d2s.md` §6 rules 2–3).
fn first_talk_bits(class: u16) -> [u8; 8] {
    let mut q = crate::world::quests::PlayerQuests::default();
    q.hear(0, class);
    q.first_talk[0]
}
