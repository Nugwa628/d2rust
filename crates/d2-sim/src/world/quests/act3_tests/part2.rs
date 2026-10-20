// Spec: specs/world/quests-act3-2.md §11 (Test vectors, edge cases)
//! The Act III clarifications of part 2 (QC-1 … QC-5): bit 17.3, the
//! progression on Mephisto's credit, the `&level` output, the monster
//! tests, the orb's weapon, the dead map-AI branches and the init-record
//! positions and rooms.

use super::*;
use act3::{DropSource, HandItem, InitPoint, Timer};

const P3: UnitId = UnitId(3);
const MEPHISTO_U: UnitId = UnitId(0x30);
const MON: UnitId = UnitId(0x31);
const DECOY: UnitId = UnitId(0x40);
const ALTAR: UnitId = UnitId(0x41);
const BOSS: UnitId = UnitId(0x42);
const ORB_U: UnitId = UnitId(0x43);
const WANDERER: UnitId = UnitId(0x60);
const SPAWNED: UnitId = UnitId(0x50);
const R1: RoomId = RoomId(1);
const R2: RoomId = RoomId(2);
const WILL: [u8; 4] = *b"qf2 ";

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

fn ev(event: u8, target: Option<UnitId>, player: UnitId, a: u32, b: u32) -> EventArgs {
    EventArgs {
        event,
        target,
        player: Some(player),
        a,
        b,
    }
}

// ------------------------------------------------------------ §11.1

// Covers: specs/world/quests-act3-2.md §11.1; specs/world/quests-act3.md §edge-cases-original-bugs r18
#[test]
fn bit_17_3_is_never_set() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.add_player(P2, 75);
    // Alkor 549 (state 2, the flag iterate), a level change out of the
    // docks (state 3, the iterate again), the tome picked up and dropped,
    // then 564: no path sets 17.3.
    call(
        &mut ctl,
        &mut f,
        15,
        ev(event::SCROLL_MESSAGE, Some(ALKOR_U), P1, 254, 549),
    );
    let i = ctl.find(15).unwrap();
    assert_eq!(ctl.records[i].state, 2);
    call(
        &mut ctl,
        &mut f,
        15,
        ev(event::CHANGED_LEVEL, Some(P2), P2, 75, 76),
    );
    assert_eq!(ctl.records[i].state, 3);
    call(
        &mut ctl,
        &mut f,
        15,
        ev(event::ITEM_PICKED_UP, None, P1, 0, 0),
    );
    call(
        &mut ctl,
        &mut f,
        15,
        ev(event::ITEM_DROPPED, None, P1, 0, 0),
    );
    f.p(P1).items = vec![*b"bbb "];
    call(
        &mut ctl,
        &mut f,
        15,
        ev(event::SCROLL_MESSAGE, Some(ALKOR_U), P1, 254, 564),
    );
    for p in [P1, P2] {
        assert!(f.flags(p).get(17, bit::STARTED) || f.flags(p).get(17, 0));
        assert!(!f.flags(p).get(17, 3), "{p:?}");
    }
    // Only a save already carrying 17.3 reaches the event-13 branch.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.p(P1).quests.flags[0].set(17, 3);
    call(
        &mut ctl,
        &mut f,
        15,
        ev(event::PLAYER_STARTED_GAME, Some(P1), P1, 0, 0),
    );
    let r = ctl.record(15).unwrap();
    assert_eq!((r.state, r.status), (3, 1));
}

// ------------------------------------------------------------ §11.2

fn mephisto_credit(difficulty: u8, flags: u16) -> (Fake3, Vec<String>) {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.difficulty = difficulty;
    f.p(P1).level = Some(102);
    monster(&mut f, MEPHISTO_U, act3::npc::MEPHISTO);
    f.f.client_flags.insert(P1, flags);
    call(
        &mut ctl,
        &mut f,
        20,
        ev(event::MONSTER_KILLED, Some(MEPHISTO_U), P1, 0, 0),
    );
    assert!(f.flags(P1).get(22, bit::PRIMARY_GOAL_DONE));
    let prog = f
        .log()
        .into_iter()
        .filter(|l| l.starts_with("progression"))
        .collect();
    (f, prog)
}

// Covers: specs/world/quests-act3-2.md §11.2
#[test]
fn mephisto_credit_progression() {
    // Vector: expansion character (bit 5), hell, progression 12 → 13
    // (5·2 + 3).
    let (f, prog) = mephisto_credit(2, 0x0C20);
    assert_eq!(prog, ["progression 1 0x0d20"]);
    assert_eq!(f.f.client_flags[&P1], 0x0D20);
    // Vector: classic, normal, progression 5: unchanged (n = 3 < 5).
    let (f, prog) = mephisto_credit(0, 0x0500);
    assert_eq!(prog, ["progression 1 0x0500"]);
    assert_eq!(f.f.client_flags[&P1], 0x0500);
    // Classic nightmare from 0: 4·1 + 3 = 7.
    let (f, _) = mephisto_credit(1, 0);
    assert_eq!(f.f.client_flags[&P1], 0x0700);
    // No client: nothing written, nothing reported.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    monster(&mut f, MEPHISTO_U, act3::npc::MEPHISTO);
    call(
        &mut ctl,
        &mut f,
        20,
        ev(event::MONSTER_KILLED, Some(MEPHISTO_U), P1, 0, 0),
    );
    assert!(!f.log().iter().any(|l| l.starts_with("progression")));
    assert!(!f.log().iter().any(|l| l.contains("0x538680")));
}

// ------------------------------------------------------------ §11.3

// Covers: specs/world/quests-act3-2.md §11.3, §edge-cases-original-bugs r3
#[test]
fn drop_level_output() {
    // Monster: its total stat 12; player: base stat 12; object: the
    // area level.
    assert_eq!(act3::drop_item_level(DropSource::Monster { level: 30 }), 30);
    assert_eq!(act3::drop_item_level(DropSource::Player { level: 12 }), 12);
    assert_eq!(
        act3::drop_item_level(DropSource::Other { area_level: 85 }),
        85
    );
    // ≤ 1 or no source → 1.
    assert_eq!(act3::drop_item_level(DropSource::Monster { level: 0 }), 1);
    assert_eq!(
        act3::drop_item_level(DropSource::Other { area_level: -4 }),
        1
    );
    assert_eq!(act3::drop_item_level(DropSource::None), 1);
}

// ------------------------------------------------------------ §11.4

fn bird_setup() -> (QuestControl, Fake3) {
    let (ctl, _) = control();
    let mut f = Fake3::new();
    monster(&mut f, MON, 100);
    f.f.chains.insert(MON, QuestChain::default());
    f.acts.insert(MON, 2);
    (ctl, f)
}

// Covers: specs/world/quests-act3-2.md §11.4
#[test]
fn bird_boss_never_flying() {
    // Vector: a flying monster (`flying` = monstats byte +0x0D & 0x40),
    // chain 18 ready: not linked; +0x01 stays 1.
    let (mut ctl, mut f) = bird_setup();
    assert_eq!(act3::q4::FLYING_0D, 0x40);
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, Some(0x40));
    let e = &ctl.record(18).unwrap().extra.act3.q4;
    assert!(e.may_choose && !e.chosen);
    assert!(f.f.chains[&MON].0.is_empty());
    // The other bits of the byte do not matter.
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, Some(0xBF));
    assert!(ctl.record(18).unwrap().extra.act3.q4.chosen);
    // No monstats row: the whole test fails (`0x00544ED3`), nothing is
    // chosen or linked, nothing reported.
    let (mut ctl, mut f) = bird_setup();
    act3::choose_bird_boss(&mut ctl, &mut f, MON, 100, None);
    assert!(!ctl.record(18).unwrap().extra.act3.q4.chosen);
    assert!(f.f.chains.get(&MON).is_none_or(|c| c.0.is_empty()));
    assert!(unhandled(&f).is_empty());
}

// Covers: specs/world/quests-act3-2.md §11.4; specs/world/quests-act3.md §edge-cases-original-bugs r19
#[test]
fn gidbinn_kill_test_flags() {
    // Type flags & 0x0E (superunique 2, champion 4, unique 8), or the
    // monstats `boss` column.
    assert!(act3::gidbinn_kill_test(0x02, false));
    assert!(act3::gidbinn_kill_test(0x04, false));
    assert!(act3::gidbinn_kill_test(0x08, false));
    assert!(act3::gidbinn_kill_test(0x00, true));
    assert!(!act3::gidbinn_kill_test(0x01, false));
    assert!(!act3::gidbinn_kill_test(0xF1, false));
    // Vector: the Gidbinn boss as a champion (type flags 1, 4 and 8):
    // `g33 ` dropped.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    monster(&mut f, BOSS, act3::npc::FETISH11);
    f.f.chains.insert(BOSS, QuestChain(vec![17]));
    let i = ctl.find(17).unwrap();
    ctl.records[i].callbacks |= 1 << event::MONSTER_KILLED;
    ctl.records[i].extra.act3.q3.boss_spawned = true;
    f.special = act3::gidbinn_kill_test(0x01 | 0x04 | 0x08, false);
    ctl.monster_killed(&mut f, BOSS, Some(P1));
    assert_eq!(f.log(), ["qdrop 66 g33  2 false"]);
    assert!(ctl.records[i].extra.act3.q3.gidbinn_dropped);
}

// ------------------------------------------------------------ §11.5

fn hand(guid: u32, weap: bool, code: [u8; 4]) -> Option<HandItem> {
    Some(HandItem { guid, weap, code })
}

// Covers: specs/world/quests-act3-2.md §11.5, §edge-cases-original-bugs r2
#[test]
fn orb_weapon_in_use() {
    // Left hand (location 5) first, then right (4); each must be `weap`
    // with the GUID +0x1C.
    let will = hand(7, true, WILL);
    let sword = hand(9, true, *b"lsd ");
    assert_eq!(act3::weapon_in_use(7, None, will), will);
    assert_eq!(act3::weapon_in_use(7, will, sword), will);
    assert_eq!(act3::weapon_in_use(9, will, sword), sword);
    // Not `weap` → the other hand.
    assert_eq!(act3::weapon_in_use(7, hand(7, false, WILL), will), will);
    assert_eq!(act3::weapon_in_use(7, hand(7, false, WILL), None), None);
    // +0x1C = −1 or no match: none.
    assert_eq!(act3::weapon_in_use(u32::MAX, will, will), None);
    assert_eq!(act3::weapon_in_use(8, will, sword), None);

    // Vector: the Will at body location 4 with +0x1C = its GUID →
    // accepted (counts as a hit).
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.objects.insert(ORB_U, (0x43, 404, 0));
    if let Some(h) = act3::weapon_in_use(7, None, will) {
        f.weapons.insert(P1, h.code);
    }
    act3::orb_operate(&mut ctl, &mut f, ORB_U, P1);
    assert_eq!(ctl.record(19).unwrap().extra.act3.q5.hits, 1);
    assert!(f.log().is_empty());
    // Vector: the Will at location 11 (swap): never consulted → sound 19.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.objects.insert(ORB_U, (0x43, 404, 0));
    if let Some(h) = act3::weapon_in_use(7, None, None) {
        f.weapons.insert(P1, h.code);
    }
    act3::orb_operate(&mut ctl, &mut f, ORB_U, P1);
    assert_eq!(f.log(), ["sound 1 19"]);
    assert_eq!(ctl.record(19).unwrap().extra.act3.q5.hits, 0);
}

// ------------------------------------------------------------ §11.6

// Covers: specs/world/quests-act3-2.md §11.6; specs/world/quests-act3.md §edge-cases-original-bugs r20
#[test]
fn map_ai_branches_are_dead() {
    // Nothing in Act III stores +0x18 (chain 14) or +0x24 (chain 20), so
    // the spawns never apply a map AI and +0x10 / +0x30 stay 0.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    ctl.game.set(16, bit::PRIMARY_GOAL_DONE);
    f.f.spawns = vec![Some(SPAWNED), Some(UnitId(0x51))];
    let at = InitPoint {
        room: R1,
        x: 30,
        y: 40,
    };
    act3::hratli_end_init(&mut ctl, &mut f, at);
    f.f.objects.insert(UnitId(0x44), (0x44, 382, 0));
    f.f.pos.insert(UnitId(0x44), (5, 6, R1));
    act3::natalya_init(&mut ctl, &mut f, UnitId(0x44));
    let q0 = &ctl.record(14).unwrap().extra.act3.q0;
    let q6 = &ctl.record(20).unwrap().extra.act3.q6;
    assert!(q0.end_spawned && !q0.map_ai && !q0.ai_applied);
    assert!(q6.natalya_spawned && !q6.map_ai && !q6.ai_applied);
    assert!(unhandled(&f).is_empty());
}

// ------------------------------------------------------------ §11.7

// Covers: specs/world/quests-act3-2.md §11.7 r1
#[test]
fn decoy_uses_its_init_record() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.objects.insert(DECOY, (0x40, 252, 0));
    // The unit's own position is not read.
    f.f.pos.insert(DECOY, (1, 2, R2));
    let at = InitPoint {
        room: R1,
        x: 100,
        y: 200,
    };
    ctl.record_mut(17).unwrap().extra.act3.q3.decoy_active = true;
    f.f.spawns.push(Some(BOSS));
    act3::decoy_init(&mut ctl, &mut f, DECOY, at);
    let e = &ctl.record(17).unwrap().extra.act3.q3;
    assert_eq!((e.decoy_x, e.decoy_y), (100, 200));
    assert!(e.decoy_known && e.boss_spawned);
    assert_eq!(f.log(), ["spawn in room 1 407"]);
    // The timer spawns in the room covering (+0x08, +0x0C), not the
    // player's.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.objects.insert(DECOY, (0x40, 252, 0));
    act3::decoy_init(&mut ctl, &mut f, DECOY, at);
    act3::decoy_operate(&mut ctl, &mut f, DECOY, P1);
    f.room_covering = Some(R2);
    f.player_in_rooms = true;
    f.f.pos.insert(P1, (500, 500, RoomId(9)));
    f.f.spawns.push(Some(BOSS));
    f.f.log.clear();
    assert!(act3::run_timer(&mut ctl, &mut f, Timer::GidbinnBoss, 17));
    assert_eq!(f.log(), ["room covering 100 200", "spawn in room 2 407"]);
}

// Covers: specs/world/quests-act3-2.md §11.7 r2
#[test]
fn altar_init_record_and_missing_unit() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    f.f.objects.insert(ALTAR, (0x41, 251, 0));
    let at = InitPoint {
        room: R1,
        x: 300,
        y: 400,
    };
    act3::altar_init(&mut ctl, &mut f, ALTAR, at);
    let e = &ctl.record(17).unwrap().extra.act3.q3;
    assert_eq!((e.altar_guid, e.altar_x, e.altar_y), (0x41, 300, 400));
    // The altar unit gone: +0x06 cleared, nothing else; +0x2C keeps 0.
    f.f.objects.remove(&ALTAR);
    ctl.record_mut(17).unwrap().extra.act3.q3.altar_ready = true;
    f.f.log.clear();
    act3::activate_altar(&mut ctl, &mut f);
    let e = &ctl.record(17).unwrap().extra.act3.q3;
    assert!(!e.altar_ready);
    assert_eq!(e.altar_mode, 0);
    assert!(f.log().is_empty());
}

// Covers: specs/world/quests-act3-2.md §11.7 r3
#[test]
fn hratli_dummies_spawn_at_the_init_record() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let at = InitPoint {
        room: R1,
        x: 30,
        y: 40,
    };
    // Init 50 stores the point before the game-flag test.
    act3::hratli_end_init(&mut ctl, &mut f, at);
    let q0 = &ctl.record(14).unwrap().extra.act3.q0;
    assert_eq!((q0.end_x, q0.end_y), (30, 40));
    assert!(f.log().is_empty());
    // Init 49: Hratli at (room, x, y), mode 1, −1.
    f.f.spawns = vec![Some(SPAWNED)];
    act3::hratli_start_init(&mut ctl, &mut f, at);
    assert_eq!(f.log(), ["spawn 253 30 40 mode 1 r 4294967295"]);
    assert!(unhandled(&f).is_empty());
}

// Covers: specs/world/quests-act3-2.md §11.7 r4; specs/world/quests-act3.md §edge-cases-original-bugs r21
#[test]
fn roomless_wanderer_spawn_retries() {
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    let at = InitPoint {
        room: R1,
        x: 100,
        y: 200,
    };
    let seed = ctl.seed;
    // No room holds (x + 7, y): no spawn, no draw, +0x01 stays 1,
    // +0x00 := 1, the target is stored.
    act3::wanderer_init(&mut ctl, &mut f, at);
    let x = ctl.record(28).unwrap().extra.act3.q7.clone();
    assert!(x.to_spawn && x.seen && (x.target_x, x.target_y) == (107, 200));
    assert!(f.log().is_empty());
    assert_eq!(ctl.seed, seed);
    // A later init 43 finds a room and spawns.
    f.f.rooms.insert(R1, (0, 0, 1000, 1000));
    f.f.spawns = vec![Some(WANDERER)];
    act3::wanderer_init(&mut ctl, &mut f, at);
    assert!(!ctl.record(28).unwrap().extra.act3.q7.to_spawn);
    assert_eq!(f.log(), ["spawn 368 107 200 mode 1 r 4294967295"]);
}

// Covers: specs/world/quests-act3-2.md §11.7 r5, §edge-cases-original-bugs r1
#[test]
fn roomless_wanderer_walk_target() {
    // Vector: the wanderer has no room → (X, Y − 20), no collision test.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    monster(&mut f, WANDERER, act3::npc::DARK_WANDERER);
    {
        let x = &mut ctl.record_mut(28).unwrap().extra.act3.q7;
        x.seen = true;
        (x.target_x, x.target_y) = (107, 200);
    }
    f.blocked.insert((107, 180));
    assert_eq!(
        act3::wanderer_target(&mut ctl, &mut f, WANDERER),
        Some((107, 180))
    );
    assert!(f.log().is_empty());
    let x = &ctl.record(28).unwrap().extra.act3.q7;
    assert!(x.target_fixed && (x.target_x, x.target_y) == (107, 180));
}

// Covers: specs/world/quests-act3-2.md §11.7 r6
#[test]
fn roomless_wanderer_minions() {
    // No room: no spot, no dummy; the bits, +0x0C, +0x0D and the draw
    // still happen.
    let (mut ctl, _) = control();
    let mut f = Fake3::new();
    monster(&mut f, WANDERER, act3::npc::DARK_WANDERER);
    f.add_player(P3, 76);
    f.f.spot = Some((0, 0));
    ctl.seed = Seed::new(12345, 666);
    {
        let x = &mut ctl.record_mut(28).unwrap().extra.act3.q7;
        x.timer = true;
        x.wanderer_guid = WANDERER.0;
    }
    assert!(act3::run_timer(
        &mut ctl,
        &mut f,
        Timer::WandererMinions,
        28
    ));
    let mut s = Seed::new(12345, 666);
    s.step();
    assert_eq!(ctl.seed, s);
    assert!(f.log().is_empty());
    assert!(f.flags(P1).get(32, bit::REWARD_GRANTED));
    assert!(f.flags(P3).get(32, bit::REWARD_GRANTED));
    let x = &ctl.record(28).unwrap().extra.act3.q7;
    assert!(x.minions && !x.timer);
}
