// Spec: specs/world/quests-act1-rest.md §9
//! The answers of `quests-act1-rest.md` §9 on the quests' fake world:
//! objects without a room (items 1–2), a player without a client (item
//! 4), the gibbet, tree and cain portal inits, the cain portal's event 7
//! and Wirt's body's operate (items 8–11).
use super::tests::*;
use super::*;
use crate::units::RoomId as RoomIdT;

const GIBBET: UnitId = UnitId(0x50);
const TREE: UnitId = UnitId(0x51);
const PORTAL: UnitId = UnitId(0x52);
const WIRT: UnitId = UnitId(0x53);
const MARKER: UnitId = UnitId(0x30);
const C1: UnitId = UnitId(0x72);
const C2: UnitId = UnitId(0x73);
const TRAP: UnitId = UnitId(0x60);

fn q4(ctl: &QuestControl) -> &act1::q4::Extra4 {
    &ctl.record(4).unwrap().extra.q4
}

fn q4_mut(ctl: &mut QuestControl) -> &mut act1::q4::Extra4 {
    &mut ctl.record_mut(4).unwrap().extra.q4
}

fn drop_chain4(ctl: &mut QuestControl) {
    ctl.records.retain(|r| r.chain != 4);
}

fn player(guid: u32, level: Option<u32>) -> Player {
    Player {
        guid,
        act: Some(0),
        level,
        ..Player::default()
    }
}

// ------------------------------------------------------------ items 1–2

/// The gibbet (object 26) at (20, 30), left in a freed room.
fn roomless_gibbet() -> Fake {
    let mut f = Fake::new();
    f.frame = 100;
    f.objects.insert(GIBBET, (0x50, 26, 0));
    f.xy.insert(GIBBET, (20, 30));
    f.spot = Some((1, 1));
    f.spawns = vec![Some(UnitId(0x15))];
    f
}

// Covers: specs/world/quests-act1-rest.md §9 r1, §9 r2, §edge-cases-original-bugs r7
#[test]
fn roomless_gibbet_event() {
    // Nobody in Tristram: no spawn try and no free-spot search (both are
    // null for a null room), no portal; Cain failed, the town Cain is
    // due; the rest of §1.2 runs (L1 broadcast with status 6).
    let (mut ctl, _) = control();
    let mut f = roomless_gibbet();
    ctl.record_mut(4).unwrap().state = 5;
    act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
    assert_eq!(f.log, ["mode 80 3"]);
    let x = q4(&ctl);
    assert!(x.cain_failed && x.town_cain_due && !x.out_portal);
    assert_eq!(x.found_player, None);
    assert_eq!(ctl.record(4).unwrap().status, 6);
    assert!(ctl.faults.is_empty());

    // The town portal already made (+0x66): no portal call, so no fault
    // even with a player in Tristram.
    let (mut ctl, _) = control();
    let mut f = roomless_gibbet();
    f.p(P1).level = Some(38);
    ctl.record_mut(4).unwrap().state = 5;
    q4_mut(&mut ctl).out_portal = true;
    act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
    assert!(ctl.faults.is_empty());
    assert!(q4(&ctl).cain_failed);

    // A player in Tristram and no portal yet: the portal creation with a
    // null room is the internal-error exit `0x0056D147`.
    let (mut ctl, _) = control();
    let mut f = roomless_gibbet();
    f.p(P1).level = Some(38);
    f.portals = vec![Some(UnitId(0x99))];
    ctl.record_mut(4).unwrap().state = 5;
    act1::q4::gibbet_event(&mut ctl, &mut f, GIBBET);
    assert_eq!(ctl.faults, [QuestError::Fatal(0x0056_D147)]);
    assert!(!f.log.iter().any(|l| l.starts_with("portal")));
    assert!(!q4(&ctl).out_portal);
}

// Covers: specs/world/quests-act1-rest.md §9 r2, §3 text
#[test]
fn roomless_marker_spawns_no_town_cain() {
    // The marker init with a null room registers the marker, spawns
    // nothing and draws nothing.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.objects.insert(MARKER, (0x30, 385, 0));
    f.spawns = vec![Some(UnitId(0x16))];
    q4_mut(&mut ctl).town_cain_due = true;
    act1::q4::marker_init(&mut ctl, &mut f, MARKER, None, 10, 10);
    assert!(f.log.is_empty() && ctl.faults.is_empty());
    let x = q4(&ctl);
    assert!(x.marker_known && x.marker_pos == (10, 10));
    assert!(x.town_cain_due && !x.town_cain);

    // Cain leaves Tristram with the marker in a freed room: the init runs
    // again at the marker's position, nothing spawns, no portal object.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.objects.insert(MARKER, (0x30, 385, 0));
    f.xy.insert(MARKER, (12, 14));
    f.spawns = vec![Some(UnitId(0x16))];
    {
        let x = q4_mut(&mut ctl);
        (x.marker_known, x.marker_guid) = (true, 0x30);
    }
    act1::q4::cain_leaves_tristram(&mut ctl, &mut f);
    assert!(f.log.is_empty() && ctl.faults.is_empty());
    let x = q4(&ctl);
    assert!(x.b91 && x.town_cain_due && !x.town_cain && !x.cain_portal);
    assert_eq!(x.marker_pos, (12, 14));
}

/// Two chests: C1 at (50, 60) left in a freed room, C2 at (70, 80) in
/// room 2; the Countess's death position (7, 8) in room 1; killed.
fn chests(ctl: &mut QuestControl, order: [u32; 2]) -> Fake {
    let mut f = Fake::new();
    f.rooms.insert(RoomIdT(1), (0, 0, 40, 40));
    f.rooms.insert(RoomIdT(2), (40, 40, 100, 100));
    f.objects.insert(C1, (0x72, 0x173, 0));
    f.xy.insert(C1, (50, 60));
    f.objects.insert(C2, (0x73, 0x173, 0));
    f.pos.insert(C2, (70, 80, RoomIdT(2)));
    let x = &mut ctl.record_mut(5).unwrap().extra.q5;
    x.chests = order.to_vec();
    x.killed = true;
    x.death_pos = (7, 8);
    f.spawns = vec![Some(TRAP)];
    f
}

// Covers: specs/world/quests-act1-rest.md §9 r1, §9 r2, §4 text
#[test]
fn trap_with_a_roomless_chest() {
    // The roomless chest first: no room lookup succeeds, no spawn, no
    // missile for it; the next chest spawns T and gets its missile.
    let (mut ctl, _) = control();
    let mut f = chests(&mut ctl, [0x72, 0x73]);
    act1::q5::chest_event(&mut ctl, &mut f, C2);
    assert_eq!(
        f.log,
        [
            "spawn 326 7 8 room 1 mode 12 spread -1 flags 0x8",
            "missile 332 owner 96 skill 0 level 1 70 80",
            "missile data 36865 0x73 0",
            "refresh 36865",
        ]
    );
    assert!(ctl.faults.is_empty());

    // T made first: the roomless chest still gets its missile at its own
    // position (its room is not read once T exists).
    let (mut ctl, _) = control();
    let mut f = chests(&mut ctl, [0x73, 0x72]);
    act1::q5::chest_event(&mut ctl, &mut f, C2);
    assert_eq!(
        f.log[4..],
        [
            "missile 332 owner 96 skill 0 level 1 50 60",
            "missile data 36866 0x72 0",
            "refresh 36866",
        ]
    );
    assert!(ctl.faults.is_empty());
}

// ------------------------------------------------------------ item 4

// Covers: specs/world/quests-act1-rest.md §9 r4, §5 text
#[test]
fn a1q6_credit_without_a_client_is_fatal() {
    // Andariel killed by P1 (no client save flags): the credit sets 6.13
    // and 6.1, then the progression reads a null client.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    let andariel = UnitId(0x44);
    f.chains.insert(andariel, QuestChain(vec![6]));
    ctl.record_mut(6).unwrap().state = 3;
    ctl.monster_killed(&mut f, andariel, Some(P1));
    assert!(ctl.faults.contains(&QuestError::Fatal(0x0053_8684)));
    assert!(f.flags(P1).get(6, bit::REWARD_PENDING));
    // With a client: the progression is written, no fault.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.client_flags.insert(P1, 0x0020);
    f.chains.insert(andariel, QuestChain(vec![6]));
    ctl.record_mut(6).unwrap().state = 3;
    ctl.monster_killed(&mut f, andariel, Some(P1));
    assert!(!ctl.faults.contains(&QuestError::Fatal(0x0053_8684)));
    assert_eq!(f.client_flags[&P1], 0x0120);
}

// ------------------------------------------------------------ items 8–9

// Covers: specs/world/quests-act1-rest.md §9 r8
#[test]
fn gibbet_init() {
    // Not opened: mode := X +0x54 = 0; +0x48 and +0x34 set.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.objects.insert(GIBBET, (0x50, 26, 2));
    act1::q4::gibbet_init(&mut ctl, &mut f, GIBBET);
    assert_eq!(f.log, ["mode 80 0"]);
    let x = q4(&ctl);
    assert!(x.gibbet_known && x.gibbet_guid == 0x50);
    // Opened (+0x54 = 3): mode 3.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.objects.insert(GIBBET, (0x50, 26, 0));
    q4_mut(&mut ctl).gibbet_open = 3;
    act1::q4::gibbet_init(&mut ctl, &mut f, GIBBET);
    assert_eq!(f.log, ["mode 80 3"]);
    // No chain 4 record: mode 2 unless it is 2.
    let (mut ctl, _) = control();
    drop_chain4(&mut ctl);
    let mut f = Fake::new();
    f.objects.insert(GIBBET, (0x50, 26, 0));
    act1::q4::gibbet_init(&mut ctl, &mut f, GIBBET);
    act1::q4::gibbet_init(&mut ctl, &mut f, GIBBET);
    assert_eq!(f.log, ["mode 80 2"]);
}

// Covers: specs/world/quests-act1-rest.md §9 r9
#[test]
fn tree_init() {
    // Live quest, Cain not gone: +0x47 := 1, +0x58 stays 0 → mode 0;
    // the tree's GUID is not written.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.objects.insert(TREE, (0x51, 30, 2));
    act1::q4::tree_init(&mut ctl, &mut f, TREE);
    assert_eq!(f.log, ["mode 81 0"]);
    let x = q4(&ctl);
    assert!(x.tree_known && x.tree_guid == 0 && x.progress == 0);
    // Cain gone (+0x50): +0x58 := 1 → mode 1.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    q4_mut(&mut ctl).cain_gone = true;
    act1::q4::tree_init(&mut ctl, &mut f, TREE);
    assert_eq!(f.log, ["mode 81 1"]);
    assert_eq!(q4(&ctl).progress, 1);
    // Not-intro 0 (done before): mode 1.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    ctl.record_mut(4).unwrap().not_intro = false;
    act1::q4::tree_init(&mut ctl, &mut f, TREE);
    assert_eq!(f.log, ["mode 81 1"]);
    // No chain 4 record: mode 2 unless it is 2.
    let (mut ctl, _) = control();
    drop_chain4(&mut ctl);
    let mut f = Fake::new();
    f.objects.insert(TREE, (0x51, 30, 2));
    act1::q4::tree_init(&mut ctl, &mut f, TREE);
    assert!(f.log.is_empty() && !q4_present(&ctl));
}

fn q4_present(ctl: &QuestControl) -> bool {
    ctl.find(4).is_some()
}

// ------------------------------------------------------------ item 10

// Covers: specs/world/quests-act1-rest.md §9 r10, §edge-cases-original-bugs r8
#[test]
fn cain_portal_init_and_event() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.frame = 40;
    f.objects.insert(PORTAL, (0x52, 189, 0));
    act1::q4::cain_portal_init(&mut f, PORTAL);
    assert_eq!(f.log, ["mode 82 1", "event7 82 65"]);
    // In the Rogue Encampment (Act I, level 1), through the event-7
    // dispatcher: 1 → 2; then 2 counts +0x80 and moves to 3 only after
    // the sixth count (+0x92); 3 → 4; then nothing; event 7 every time.
    f.unit_levels.insert(PORTAL, 1);
    f.log.clear();
    let mut modes = Vec::new();
    for _ in 0..10 {
        act1::q4::cain_portal_event(&mut ctl, &mut f, PORTAL);
        modes.push(f.objects[&PORTAL].2);
    }
    assert_eq!(modes, [2, 2, 2, 2, 2, 2, 3, 4, 4, 4]);
    let x = q4(&ctl);
    assert!(x.town_portal_count == 6 && x.b92);
    assert_eq!(f.log.iter().filter(|l| *l == "event7 82 65").count(), 10);
    // Another portal in town: the shared counter is past 5, so mode 2
    // goes to 3 at once (+0x80 never reset).
    f.objects.insert(PORTAL, (0x52, 189, 2));
    act1::q4::cain_portal_event(&mut ctl, &mut f, PORTAL);
    assert_eq!(f.objects[&PORTAL].2, 3);
    assert_eq!(q4(&ctl).town_portal_count, 7);

    // Outside the Rogue Encampment: mode 2 → 3 only once +0x91 is set.
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.objects.insert(PORTAL, (0x52, 189, 2));
    f.unit_levels.insert(PORTAL, 38);
    act1::q4::cain_portal_event(&mut ctl, &mut f, PORTAL);
    assert_eq!(f.objects[&PORTAL].2, 2);
    q4_mut(&mut ctl).b91 = true;
    act1::q4::cain_portal_event(&mut ctl, &mut f, PORTAL);
    assert_eq!(f.objects[&PORTAL].2, 3);
    assert_eq!(q4(&ctl).town_portal_count, 0);

    // No chain 4 record: nothing, not even the reschedule.
    let (mut ctl, _) = control();
    drop_chain4(&mut ctl);
    let mut f = Fake::new();
    f.objects.insert(PORTAL, (0x52, 189, 1));
    act1::q4::cain_portal_event(&mut ctl, &mut f, PORTAL);
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests.md §9.5
#[test]
fn act1_cain_portal_routes_through_the_dispatcher() {
    let (mut ctl, _) = control();
    let mut f = Fake::new();
    f.frame = 10;
    f.objects.insert(PORTAL, (0x52, 189, 1));
    // The fake answers a unit's act and level from a player entry.
    f.players.insert(PORTAL, player(0x52, Some(1)));
    object_event(&mut ctl, &mut f, PORTAL, 0xBD);
    assert_eq!(f.log, ["mode 82 2", "event7 82 35"]);
}

// ------------------------------------------------------------ item 11

// Covers: specs/world/quests-act1-rest.md §9 r11, §edge-cases-original-bugs r9
#[test]
fn wirt_body_operate() {
    // Mode 0, the drop succeeds: drop code `leg `, quality 2, not
    // droppable; mode 1, event 1 at frame + (0x1000 >> 8) + 1, event 7 at
    // frame + 10.
    let mut f = Fake::new();
    f.frame = 200;
    f.objects.insert(WIRT, (0x53, 268, 0));
    act1::q4::wirt_body_operate(&mut f, WIRT);
    assert_eq!(
        f.log,
        [
            "qdrop 83 leg  2 None false",
            "mode 83 1",
            "event1 83 217",
            "event7 83 210",
        ]
    );
    // Mode ≠ 0: nothing.
    f.log.clear();
    act1::q4::wirt_body_operate(&mut f, WIRT);
    assert!(f.log.is_empty());
    // The drop fails: mode stays 0, the next operate tries again.
    let mut f = Fake::new();
    f.objects.insert(WIRT, (0x53, 268, 0));
    f.drop_fails = true;
    act1::q4::wirt_body_operate(&mut f, WIRT);
    act1::q4::wirt_body_operate(&mut f, WIRT);
    assert_eq!(
        f.log,
        ["qdrop 83 leg  2 None false", "qdrop 83 leg  2 None false"]
    );
    assert_eq!(f.objects[&WIRT].2, 0);
}
