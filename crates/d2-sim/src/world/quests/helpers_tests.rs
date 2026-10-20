// Spec: specs/world/quests-helpers.md
// Spec: specs/world/quests-act2-2.md (§5.1, §5.4)
//! The helpers' test vectors on the shared fake world.

use super::super::tests::{hex, Fake, P1};
use super::*;
use crate::units::{RoomId, UnitId};
use crate::world::quests::UnitKind;

const R1: RoomId = RoomId(1);
const R2: RoomId = RoomId(2);

fn world() -> Fake {
    let mut f = Fake::new();
    f.rooms.insert(R1, (0, 0, 100, 100));
    f
}

// Covers: specs/world/quests-helpers.md §1; specs/world/quests-act2-2.md §edge-cases-original-bugs r4
#[test]
fn free_spot_lattice() {
    // Everything free: ring k = 1 tests the corner (48, 50) only → spot
    // (49, 51).
    let mut f = world();
    assert_eq!(
        free_spot(&mut f, R1, 50, 50, 2, 0x100, 3),
        Some((49, 51, R1))
    );
    // Limit 1: no ring at all.
    assert_eq!(free_spot(&mut f, R1, 50, 50, 2, 0x100, 1), None);
    // Only corner (49, 49) free: k = 2's last candidate (i 0, j 0).
    f.h_free = Some(vec![(49, 49)]);
    assert_eq!(
        free_spot(&mut f, R1, 50, 50, 2, 0x100, 3),
        Some((50, 50, R1))
    );
    // Nothing free within the limit.
    f.h_free = Some(Vec::new());
    assert_eq!(free_spot(&mut f, R1, 50, 50, 2, 0x100, 100), None);
}

// Covers: specs/world/quests-helpers.md §1, §edge-cases-original-bugs r1
#[test]
fn free_spot_never_accepts_outside_the_row_room() {
    // The only free corner lies outside every room: the search fails.
    let mut f = world();
    f.h_free = Some(vec![(99, 101)]);
    assert_eq!(free_spot(&mut f, R1, 100, 100, 2, 0, 3), None);
}

// Covers: specs/world/quests-helpers.md §2, §edge-cases-original-bugs r3
#[test]
fn critical_spawn_ladder() {
    // R's box (0, 0, 10, 10), x 20, y 3: the inside walk fails, so the
    // search starts at (y, y + 21) = (3, 24), inside R2 → spot (2, 25).
    let mut f = Fake::new();
    f.rooms.insert(R1, (0, 0, 10, 10));
    f.rooms.insert(R2, (0, 20, 100, 100));
    let m = UnitId(0x55);
    f.spawns = vec![None, Some(m)];
    assert_eq!(critical_spawn(&mut f, R1, 20, 3, 512), Some(m));
    assert_eq!(
        f.log,
        [
            "spawn 512 2 25 room 2 mode 1 spread -1 flags 0x0",
            "spawn 512 2 25 room 2 mode 1 spread 5 flags 0x0",
            "flags 85 0x3000000",
        ]
    );
    // Every try fails: −1, 5, then 20 tries with spread 10, then 15 at
    // (R, x, y); nothing returned, no flags.
    f.log.clear();
    f.spawns.clear();
    f.h_free = Some(Vec::new());
    assert_eq!(critical_spawn(&mut f, R1, 20, 3, 512), None);
    assert_eq!(f.log.len(), 23);
    assert_eq!(f.log[0], "spawn 512 20 3 room 1 mode 1 spread -1 flags 0x0");
    assert!(f.log[2..22].iter().all(|l| l.contains("spread 10")));
    assert_eq!(
        f.log[22],
        "spawn 512 20 3 room 1 mode 1 spread 15 flags 0x0"
    );
}

// Covers: specs/world/quests-helpers.md §3
#[test]
fn superunique_at_point() {
    let mut f = world();
    let dummy = UnitId(0x70);
    f.pos.insert(dummy, (5, 5, R1));
    f.a5_crits = vec![Some(UnitId(0x71))];
    assert_eq!(
        spawn_superunique(&mut f, dummy, 7, 8, 2, 42),
        Some(UnitId(0x71))
    );
    assert_eq!(f.log, ["superunique 42 7 8 room 1"]);
    // No room at the point: nothing.
    f.log.clear();
    assert_eq!(spawn_superunique(&mut f, dummy, 500, 8, 2, 42), None);
    assert!(f.log.is_empty());
    // Kind 0 (no quest caller) is reported.
    assert_eq!(spawn_superunique(&mut f, dummy, 7, 8, 0, 42), None);
    assert_eq!(f.log, ["unhandled 254 0x659b80"]);
}

// Covers: specs/world/quests-helpers.md §4.1, §4.2; specs/world/quests-act1-rest.md §4.1
#[test]
fn quest_missiles() {
    let mut f = world();
    let (victim, statue) = (UnitId(0x60), UnitId(0x61));
    f.objects.insert(statue, (0x61, 475, 0));
    f.pos.insert(victim, (10, 10, R1));
    f.pos.insert(statue, (30, 40, R1));
    let m = quest_missile(&mut f, victim, statue, 541, 0x420, 1);
    assert_eq!(m, Some(UnitId(0x9001)));
    assert_eq!(
        f.log,
        [
            "spawn missile 541 owner 96 origin Some(96) flags 0x420 at 0 0 target 30 40 skill 0 level 1",
            "missile guid 36865 0x61",
        ]
    );
    // §4.2: distance 190 > 100 → none; within range → flags 1 record.
    f.log.clear();
    assert_eq!(missile_at_point(&mut f, victim, 0, 1, 625, 200, 10), None);
    assert!(f.log.is_empty());
    assert!(missile_at_point(&mut f, victim, 0, 1, 625, 10, 10).is_some());
    assert_eq!(
        f.log,
        ["spawn missile 625 owner 96 origin None flags 0x1 at 10 10 target 0 0 skill 0 level 1"]
    );
    // (0, 0) and no path target: none.
    f.log.clear();
    assert_eq!(missile_at_point(&mut f, victim, 0, 1, 625, 0, 0), None);
    assert_eq!(f.log, ["unhandled 254 0x56d2c0"]);
}

// Covers: specs/world/quests-helpers.md §4.3
#[test]
fn orb_missile_and_its_fallback() {
    let (mut ctl, _) = super::super::tests::control();
    let mut f = world();
    let (orb, stairs) = (UnitId(0x41), UnitId(0x81));
    f.pos.insert(orb, (10, 20, R1));
    f.pos.insert(stairs, (12, 25, R1));
    f.objects.insert(stairs, (0x81, 386, 0));
    // No class-386 object in the room list: fatal 0x8F.
    orb_missile(&mut ctl, &mut f, orb);
    assert_eq!(ctl.faults, [QuestError::Fatal(0x005D_FEE0)]);
    ctl.faults.clear();
    f.a5_adjacent.insert(R1, vec![orb, stairs]);
    // Not made and the stairs in mode 0: mode 1 with ENDANIM at frame +
    // (FrameCnt1 >> 8).
    f.h_missile_fails = true;
    f.frame = 100;
    orb_missile(&mut ctl, &mut f, orb);
    assert_eq!(&f.log[1..], ["mode 129 1", "event1 129 116"]);
    // Made: data +0x28 := the stairs' GUID, its room refreshed.
    f.h_missile_fails = false;
    f.log.clear();
    orb_missile(&mut ctl, &mut f, orb);
    assert_eq!(&f.log[1..], ["missile guid 36865 0x81", "refresh 129"]);
    assert!(ctl.faults.is_empty());
}

// Covers: specs/world/quests-helpers.md §5, §edge-cases-original-bugs r4
#[test]
fn end_interaction_by_kind() {
    let npc = UnitId(0x07);
    let mut f = world();
    f.monsters.insert(
        npc,
        (
            7,
            148,
            UnitKind::Monster {
                class: 148,
                superunique: None,
                owner: None,
            },
        ),
    );
    // NPC GUID 7: interaction reset, chat node freed, `62 01 07000000`.
    f.interact.insert(P1, (1, 7));
    end_interaction(&mut f, P1);
    assert_eq!(f.log, ["interact 1 None", "free chat 7 1"]);
    assert_eq!(f.sent, [(P1, hex("62 01 07000000 00"))]);
    // No interaction: kind 6, GUID 0.
    f.log.clear();
    f.sent.clear();
    end_interaction(&mut f, P1);
    assert!(f.log.is_empty());
    assert_eq!(f.sent, [(P1, hex("62 06 00000000 00"))]);
    // Trade (kind 0): the 0x4F body with button 6, no 0x62.
    f.sent.clear();
    f.players.get_mut(&P1).unwrap().guid = 1;
    f.interact.insert(P1, (0, 1));
    end_interaction(&mut f, P1);
    assert_eq!(f.log, ["trade button 1 6"]);
    assert!(f.sent.is_empty());
    // The orifice (object 152): mode 0, reset, `62 02 <guid>`.
    f.log.clear();
    let orifice = UnitId(0x90);
    f.objects.insert(orifice, (0x90, 152, 1));
    f.interact.insert(P1, (2, 0x90));
    end_interaction(&mut f, P1);
    assert_eq!(f.log, ["mode 144 0", "interact 1 None"]);
    assert_eq!(f.sent, [(P1, hex("62 02 90000000 00"))]);
    // An item: the cube is closed; another item does nothing.
    f.log.clear();
    f.sent.clear();
    let cube = UnitId(0x300);
    f.item_codes.insert(cube, *b"box ");
    f.interact.insert(P1, (4, 0x300));
    end_interaction(&mut f, P1);
    assert_eq!(f.log, ["interact 1 None", "close cube 1"]);
    assert_eq!(f.sent, [(P1, hex("62 04 00030000 00"))]);
    f.log.clear();
    f.sent.clear();
    f.item_codes.insert(cube, *b"hst ");
    f.interact.insert(P1, (4, 0x300));
    end_interaction(&mut f, P1);
    assert!(f.log.is_empty() && f.sent.is_empty());
}

// Covers: specs/world/quests-helpers.md §7
#[test]
fn town_portal_close() {
    let (mut ctl, _) = super::super::tests::control();
    let mut f = world();
    let tp = UnitId(0x90);
    // Not a town portal (class ≠ 59): nothing.
    f.h_tp.insert(P1, 0x90);
    f.objects.insert(tp, (0x90, 60, 0));
    close_town_portal(&mut ctl, &mut f, P1);
    assert!(f.log.is_empty());
    // A town portal without a partner: the chain-35 hook, then freed.
    let i = ctl.find(35).unwrap();
    ctl.records[i].extra.a5.q5.portals = 1;
    f.objects.insert(tp, (0x90, 59, 0));
    close_town_portal(&mut ctl, &mut f, P1);
    assert_eq!(f.log, ["free portal 144"]);
    assert_eq!(ctl.records[i].extra.a5.q5.portals, 0);
}

// Covers: specs/world/quests-act2-2.md §5.1
#[test]
fn quest_chest_gate_modes() {
    let mut f = world();
    let chest = UnitId(0x95);
    f.frame = 10;
    // Already open: 0, nothing done.
    f.objects.insert(chest, (0x95, 355, 1));
    assert!(!quest_chest_gate(&mut f, chest));
    assert!(f.log.is_empty());
    // `Mode1` set: mode 1 and ENDANIM at frame + (FrameCnt1 >> 8), no
    // + 1; flag 0x2 cleared.
    f.objects.insert(chest, (0x95, 355, 0));
    f.h_mode1.insert(chest, true);
    assert!(quest_chest_gate(&mut f, chest));
    assert_eq!(
        f.log,
        ["mode 149 1", "event1 149 26", "clear flags 149 0x2"]
    );
    // `Mode1` 0: mode 2, no event.
    f.log.clear();
    f.objects.insert(chest, (0x95, 355, 0));
    f.h_mode1.insert(chest, false);
    assert!(quest_chest_gate(&mut f, chest));
    assert_eq!(f.log, ["mode 149 2", "clear flags 149 0x2"]);
}

// Covers: specs/world/quests-act2-2.md §5.4
#[test]
fn scroll_text_bytes() {
    let m = msg_scroll_text(0x1234_5678, 396);
    assert_eq!(m.len(), 40);
    assert_eq!(&m[..12], hex("27 02 78563412 01 00 00 00 8C01"));
    assert!(m[12..].iter().all(|&b| b == 0));
}

// Covers: specs/world/quests-act3.md §4.7; specs/world/quests-act5-2.md §7.8
#[test]
fn stairs_warp_every_tile() {
    let mut f = world();
    let stairs = UnitId(0x96);
    f.h_tiles.insert(stairs, vec![UnitId(0xA1), UnitId(0xA2)]);
    stairs_warp(&mut f, P1, stairs);
    assert_eq!(f.log, ["warp tile 1 161", "warp tile 1 162"]);
}
