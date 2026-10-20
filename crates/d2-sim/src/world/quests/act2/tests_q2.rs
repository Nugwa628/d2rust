// Spec: specs/world/quests-act2.md §4 (Test vectors, Edge cases 3, 4)
// Spec: specs/world/quests-act2-2.md §1 items 2, 13, 20
//! Tests for [`super::q2`]: the Horadric Staff record on the quests'
//! fake world.

use super::q2::{self, CAIN2};
use crate::rng::Seed;
use crate::units::UnitId;
use crate::world::quests::tests::*;
use crate::world::quests::*;

const CAIN_U: UnitId = UnitId(0x20);
const CHEST: UnitId = UnitId(0x40);
const ITEM: UnitId = UnitId(0x80);

/// The fake with Cain and P1 (P2 when `two`) in Lut Gholein.
fn fake(two: bool) -> Fake {
    let mut f = Fake::new();
    f.monsters.insert(
        CAIN_U,
        (
            0x20,
            CAIN2,
            UnitKind::Monster {
                class: u32::from(CAIN2),
                superunique: None,
                owner: None,
            },
        ),
    );
    if two {
        f.players.insert(
            P2,
            Player {
                guid: 2,
                ..Player::default()
            },
        );
    }
    for p in f.players() {
        f.p(p).act = Some(1);
        f.p(p).level = Some(40);
    }
    f
}

fn set(f: &mut Fake, p: UnitId, slot: u8, b: u8) {
    f.p(p).quests.flags[0].set(slot, b);
}

fn give(f: &mut Fake, p: UnitId, codes: &[&[u8; 4]]) {
    for c in codes {
        f.p(p).items.push(**c);
    }
}

/// Event `ev` to chain 9's record only.
fn ev(ctl: &mut QuestControl, f: &mut Fake, args: EventArgs) -> TextList {
    let mut list = TextList::new();
    let i = ctl.find(9).unwrap();
    super::callback(ctl, f, i, args, Some(&mut list));
    list
}

fn chat(ctl: &mut QuestControl, f: &mut Fake, npc: UnitId) -> TextList {
    let args = EventArgs {
        event: event::NPC_ACTIVATE,
        target: Some(npc),
        player: Some(P1),
        ..EventArgs::default()
    };
    ev(ctl, f, args)
}

fn active(ctl: &QuestControl, f: &mut Fake, npc: u16) -> bool {
    q2::active(ctl, f, ctl.find(9).unwrap(), P1, npc)
}

fn status(ctl: &QuestControl, f: &mut Fake, p: UnitId) -> u8 {
    let pf = f.flags(p);
    q2::status(ctl, f, ctl.find(9).unwrap(), p, &pf)
}

fn x(ctl: &QuestControl) -> &q2::Extra {
    &ctl.record(9).unwrap().extra.a2.q2
}

fn x_mut(ctl: &mut QuestControl) -> &mut q2::Extra {
    &mut ctl.record_mut(9).unwrap().extra.a2.q2
}

// ------------------------------------------------------------ §4.3, §4.4

// Covers: specs/world/quests-act2.md §4.3, §4.4
#[test]
fn cain_selector_vectors() {
    let (mut ctl, _) = control();
    // Holds `box ` (10.6 set) and `vip ` (10.4 clear): out 9, then 1,
    // true → table state 1 (message 336).
    let mut f = fake(false);
    give(&mut f, P1, &[b"box ", b"vip "]);
    set(&mut f, P1, 10, 6);
    assert_eq!(q2::selector(&f, P1, &f.flags(P1)), (1, true));
    assert_eq!(chat(&mut ctl, &mut f, CAIN_U), [(336, 0)]);
    assert!(active(&ctl, &mut f, CAIN2));
    // `tr1 ` only, 10.3 set: out 6, false → 10.3 set → table state 6.
    let mut f = fake(false);
    give(&mut f, P1, &[b"tr1 "]);
    set(&mut f, P1, 10, 3);
    assert_eq!(q2::selector(&f, P1, &f.flags(P1)), (6, false));
    assert_eq!(chat(&mut ctl, &mut f, CAIN_U), [(335, 2)]);
    assert!(!active(&ctl, &mut f, CAIN2));
    // `box ` held and 10.6 clear stops at 3 before the later codes.
    let mut f = fake(false);
    give(&mut f, P1, &[b"tr1 ", b"box "]);
    assert_eq!(q2::selector(&f, P1, &f.flags(P1)), (3, true));
    // Later assignments win: all told → `msf `'s 8.
    give(&mut f, P1, &[b"vip ", b"msf "]);
    for b in [3, 4, 5, 6] {
        set(&mut f, P1, 10, b);
    }
    assert_eq!(q2::selector(&f, P1, &f.flags(P1)), (8, false));
    // `hst ` overrides: 4 (true) with 10.10 clear, 5 (false) with it.
    give(&mut f, P1, &[b"hst "]);
    assert_eq!(q2::selector(&f, P1, &f.flags(P1)), (4, true));
    assert_eq!(chat(&mut ctl, &mut f, CAIN_U), [(339, 0)]);
    set(&mut f, P1, 10, 10);
    assert_eq!(q2::selector(&f, P1, &f.flags(P1)), (5, false));
    assert_eq!(chat(&mut ctl, &mut f, CAIN_U), [(339, 2)]);
    // Nothing held: 0xFF, false.
    let f = fake(false);
    assert_eq!(q2::selector(&f, P1, &f.flags(P1)), (0xFF, false));
}

// Covers: specs/world/quests-act2.md §4.4
#[test]
fn cain_chat_order() {
    let (mut ctl, _) = control();
    let mut f = fake(false);
    // Only Cain.
    give(&mut f, P1, &[b"vip "]);
    let other = UnitId(0x21);
    let kind = f.monsters[&CAIN_U].2;
    f.monsters.insert(other, (0x21, 176, kind));
    assert!(chat(&mut ctl, &mut f, other).is_empty());
    assert!(!active(&ctl, &mut f, 176));
    // 10.1 → 4 before the selector.
    set(&mut f, P1, 10, 1);
    assert_eq!(chat(&mut ctl, &mut f, CAIN_U), [(339, 0)]);
    assert!(active(&ctl, &mut f, CAIN2));
    // Nothing held, not listed, 10.0 clear, out 0xFF: 10.3 clear →
    // nothing, set → 6.
    let mut f = fake(false);
    assert!(chat(&mut ctl, &mut f, CAIN_U).is_empty());
    set(&mut f, P1, 10, 3);
    assert_eq!(chat(&mut ctl, &mut f, CAIN_U), [(335, 2)]);
    // 10.0 → nothing.
    set(&mut f, P1, 10, 0);
    assert!(chat(&mut ctl, &mut f, CAIN_U).is_empty());
    // GUID listed → 5, before 10.0.
    ctl.record_mut(9).unwrap().guids.add(1);
    assert_eq!(chat(&mut ctl, &mut f, CAIN_U), [(339, 2)]);
}

// ------------------------------------------------------------ §4.5

// Covers: specs/world/quests-act2.md §4.5
#[test]
fn cain_messages() {
    let (mut ctl, _) = control();
    let mut f = fake(false);
    let msg = |ctl: &mut QuestControl, f: &mut Fake, npc: u16, m: u32| {
        let args = EventArgs {
            event: event::SCROLL_MESSAGE,
            target: Some(CAIN_U),
            player: Some(P1),
            a: u32::from(npc),
            b: m,
        };
        ev(ctl, f, args);
    };
    give(&mut f, P1, &[b"tr1 "]);
    // Another NPC: nothing.
    msg(&mut ctl, &mut f, 176, 335);
    assert_eq!(f.flags(P1).word(10), 0);
    msg(&mut ctl, &mut f, CAIN2, 335);
    assert_eq!(f.log, ["delete tr1 "]);
    assert!(f.p(P1).items.is_empty());
    assert_eq!(f.flags(P1).word(10), 1 << 3);
    for (m, b) in [(336, 4), (337, 5), (338, 6)] {
        let mut f = fake(false);
        ctl.record_mut(9).unwrap().flags = 5;
        msg(&mut ctl, &mut f, CAIN2, m);
        assert_eq!(ctl.record(9).unwrap().flags, 0);
        assert_eq!(f.flags(P1).word(10), 1 << 3 | 1 << b);
        assert!(f.sent.is_empty());
    }
    let mut f = fake(false);
    set(&mut f, P1, 10, 1);
    msg(&mut ctl, &mut f, CAIN2, 339);
    let want = 1 << 3 | 1 << 4 | 1 << 5 | 1 << 6 | 1 << 10;
    assert_eq!(f.flags(P1).word(10), want);
    assert!(ctl.record(9).unwrap().guids.contains(1));
    // Refresh: 0x27 then 0x29.
    assert_eq!(f.sent_ids(), [0x27, 0x29]);
}

// ------------------------------------------------------------ §4.6

// Covers: specs/world/quests-act2.md §4.6 r1, §4.6 r2, §4.6 r3
#[test]
fn status_function() {
    let (mut ctl, _) = control();
    let mut f = fake(false);
    // r1: Act I not done.
    assert_eq!(status(&ctl, &mut f, P1), 0);
    set(&mut f, P1, 7, 0);
    // Vector: nothing held, cube dropped, cube count 0, 10.3 clear → 9.
    x_mut(&mut ctl).cube_dropped = true;
    assert_eq!(status(&ctl, &mut f, P1), 9);
    // Same with 10.3 set → 2.
    set(&mut f, P1, 10, 3);
    assert_eq!(status(&ctl, &mut f, P1), 2);
    // 10.3 clear and a piece held → 4 (overrides 9).
    let mut f = fake(false);
    set(&mut f, P1, 7, 0);
    give(&mut f, P1, &[b"vip "]);
    assert_eq!(status(&ctl, &mut f, P1), 4);
    // Nothing missing, nothing held → 0.
    let mut f = fake(false);
    set(&mut f, P1, 7, 0);
    x_mut(&mut ctl).cube_count = 1;
    assert_eq!(status(&ctl, &mut f, P1), 0);
    // The staff missing; the amulet missing only with the altar gone.
    x_mut(&mut ctl).staff_dropped = true;
    assert_eq!(status(&ctl, &mut f, P1), 9);
    x_mut(&mut ctl).staff_count = 1;
    x_mut(&mut ctl).amulet_count = 0;
    assert_eq!(status(&ctl, &mut f, P1), 0);
    ctl.record_mut(10).unwrap().extra.a2.q3.altar_destroyed = true;
    assert_eq!(status(&ctl, &mut f, P1), 9);
    // `tr1 ` with 10.3 clear → 1.
    give(&mut f, P1, &[b"tr1 "]);
    assert_eq!(status(&ctl, &mut f, P1), 1);
    // `hst ` → 6 − 10.10.
    give(&mut f, P1, &[b"hst "]);
    assert_eq!(status(&ctl, &mut f, P1), 6);
    set(&mut f, P1, 10, 10);
    assert_eq!(status(&ctl, &mut f, P1), 5);
    // All three pieces: 10.6 ? 3 : 10.3 ? 2 : 4.
    let mut f = fake(false);
    set(&mut f, P1, 7, 0);
    give(&mut f, P1, &[b"msf ", b"box ", b"vip "]);
    assert_eq!(status(&ctl, &mut f, P1), 4);
    set(&mut f, P1, 10, 3);
    assert_eq!(status(&ctl, &mut f, P1), 2);
    set(&mut f, P1, 10, 6);
    assert_eq!(status(&ctl, &mut f, P1), 3);
    // r2: 10.0 or 10.1 → by 14.0.
    let mut f = fake(false);
    set(&mut f, P1, 7, 0);
    set(&mut f, P1, 10, 1);
    assert_eq!(status(&ctl, &mut f, P1), 6);
    set(&mut f, P1, 10, 10);
    assert_eq!(status(&ctl, &mut f, P1), 5);
    set(&mut f, P1, 14, 0);
    assert_eq!(status(&ctl, &mut f, P1), 11);
    set(&mut f, P1, 10, 13);
    assert_eq!(status(&ctl, &mut f, P1), 13);
    // The machinery reports it in 0x5D (status functions, quests.md §6.3).
    f.sent.clear();
    ctl.send_status(&mut f, P1, 9).unwrap();
    assert_eq!(f.sent, [(P1, hex("5d 09 00 0d 0000"))]);
}

// ------------------------------------------------------------ §4.7

// Covers: specs/world/quests-act2.md §4.7, §1.3
// Covers: specs/world/quests-act2-2.md §1 r20
#[test]
fn scroll_chest_gold_vector() {
    let (mut ctl, _) = control();
    let mut f = fake(true);
    // P2 read the scroll: one qualifying player.
    set(&mut f, P2, 10, 3);
    ctl.seed = Seed::new(12345, 666);
    assert_eq!(q2::scroll_chest(&mut ctl, &mut f, CHEST, P1), 0);
    // The drop code is stored first (act2-2 §1 item 20; the fake has no
    // body for that seam, so the default reports it).
    let mut want = vec![
        DROP_CODE.to_string(),
        "qdrop 64 tr1  7 None true".to_string(),
        "treasure 64 4".to_string(),
    ];
    // lo' 22752887 → 22752887 mod 5 + 5 = 7 gold piles.
    want.extend((0..7).map(|_| "gold 64".to_string()));
    assert_eq!(f.log, want);
    let mut s = Seed::new(12345, 666);
    s.step();
    assert_eq!(ctl.seed, s);
    // The gate closed: nothing, no draw.
    let mut f = fake(true);
    f.gate_closed = true;
    assert_eq!(q2::scroll_chest(&mut ctl, &mut f, CHEST, P1), 0);
    assert!(f.log.is_empty());
    assert_eq!(ctl.seed, s);
    // 10.0 also disqualifies; the drop code is still stored.
    let mut f = fake(false);
    set(&mut f, P1, 10, 0);
    q2::scroll_chest(&mut ctl, &mut f, CHEST, P1);
    assert_eq!(f.log[..2], [DROP_CODE, "treasure 64 4"]);
}

/// The default body of the drop-code seam (the fake has none).
const DROP_CODE: &str = "unhandled 255 0x599c28";

// Covers: specs/world/quests-act2-2.md §1 r20
#[test]
fn chests_without_chain_9() {
    // No chain 9: the drop code, no count and no items; treasure and gold
    // still drop (one quest-seed step).
    let (mut ctl, _) = control();
    ctl.records.retain(|r| r.chain != 9);
    ctl.seed = Seed::new(12345, 666);
    for chest in [q2::scroll_chest::<Fake>, q2::staff_chest, q2::cube_chest] {
        let mut f = fake(true);
        assert_eq!(chest(&mut ctl, &mut f, CHEST, P1), 0);
        assert_eq!(f.log[..2], [DROP_CODE, "treasure 64 4"]);
        assert!(f.log[2..].iter().all(|l| l == "gold 64"));
        assert!(f.log.len() >= 7);
    }
}

// Covers: specs/world/quests-act2.md §4.7, §edge-cases-original-bugs r4
#[test]
fn staff_and_cube_chests() {
    let (mut ctl, _) = control();
    let mut f = fake(true);
    // P2 holds an `hst `: only P1 qualifies for the staff.
    give(&mut f, P2, &[b"hst "]);
    q2::staff_chest(&mut ctl, &mut f, CHEST, P1);
    assert_eq!(
        f.log[..4],
        [
            DROP_CODE,
            "qdrop 64 msf  7 None true",
            "identify 600",
            "treasure 64 4"
        ]
    );
    // Edge case 4: the Staff-of-Kings count (+0x18), not the cube's.
    assert_eq!((x(&ctl).staff_count, x(&ctl).cube_count), (1, 0));
    assert!(x(&ctl).staff_dropped && !x(&ctl).cube_dropped);
    // A failed drop changes no count.
    f.drop_fails = true;
    f.log.clear();
    q2::staff_chest(&mut ctl, &mut f, CHEST, P1);
    assert_eq!(
        f.log[..3],
        [DROP_CODE, "qdrop 64 msf  7 None true", "treasure 64 4"]
    );
    assert_eq!(x(&ctl).staff_count, 1);
    // Cube: every player without a `box `, normal quality.
    let mut f = fake(true);
    q2::cube_chest(&mut ctl, &mut f, CHEST, P2);
    assert_eq!(
        f.log[..4],
        [
            DROP_CODE,
            "qdrop 64 box  2 None true",
            "qdrop 64 box  2 None true",
            "treasure 64 4"
        ]
    );
    assert_eq!(x(&ctl).cube_count, 2);
    assert!(x(&ctl).cube_dropped);
    // Holding a `box ` or 10.0 for the staff.
    let mut f = fake(false);
    give(&mut f, P1, &[b"box "]);
    set(&mut f, P1, 10, 0);
    q2::cube_chest(&mut ctl, &mut f, CHEST, P1);
    q2::staff_chest(&mut ctl, &mut f, CHEST, P1);
    assert!(!f.log.iter().any(|l| l.starts_with("qdrop")));
}

// ------------------------------------------------------------ §4.8

fn item_ev(ctl: &mut QuestControl, f: &mut Fake, e: u8, p: UnitId, code: &[u8; 4]) {
    f.item_codes.insert(ITEM, *code);
    f.chains.insert(ITEM, QuestChain(vec![9]));
    ctl.item_event(f, e, p, ITEM);
}

// Covers: specs/world/quests-act2-2.md §1 r13
#[test]
fn pick_up_scroll_after_reading() {
    // Vector: pick-up `tr1 `, 10.3 set → flags := 0; status unchanged, no
    // 0x5D (even though F would send one).
    let (mut ctl, _) = control();
    let mut f = fake(false);
    set(&mut f, P1, 7, 0);
    set(&mut f, P1, 10, 3);
    ctl.record_mut(9).unwrap().flags = 3;
    ctl.record_mut(9).unwrap().status = 4;
    item_ev(&mut ctl, &mut f, event::ITEM_PICKED_UP, P1, b"tr1 ");
    assert_eq!(ctl.record(9).unwrap().flags, 0);
    assert_eq!(ctl.record(9).unwrap().status, 4);
    assert!(f.sent.is_empty());
}

// Covers: specs/world/quests-act2.md §4.8, §edge-cases-original-bugs r3
#[test]
fn pick_up_and_drop() {
    let (mut ctl, _) = control();
    let mut f = fake(true);
    set(&mut f, P1, 7, 0);
    set(&mut f, P2, 7, 0);
    ctl.record_mut(9).unwrap().flags = 3;
    // `tr1 ` with 10.3 clear → status 1, 0x5D to this player (status
    // function: `tr1 ` held and 10.3 clear → 1).
    give(&mut f, P1, &[b"tr1 "]);
    item_ev(&mut ctl, &mut f, event::ITEM_PICKED_UP, P1, b"tr1 ");
    assert_eq!(ctl.record(9).unwrap().status, 1);
    assert_eq!(ctl.record(9).unwrap().flags, 0);
    assert_eq!(f.sent, [(P1, hex("5d 09 00 01 0000"))]);
    // Edge case 3: P2 (10.3 set) picks up a `vip `: status 2 for everyone.
    set(&mut f, P2, 10, 3);
    f.sent.clear();
    item_ev(&mut ctl, &mut f, event::ITEM_PICKED_UP, P2, b"vip ");
    assert_eq!(ctl.record(9).unwrap().status, 2);
    assert_eq!(f.sent, [(P2, hex("5d 09 00 02 0000"))]);
    // P1 (10.3 clear) a `box ` / `msf `: 6.
    for c in [b"box ", b"msf "] {
        ctl.record_mut(9).unwrap().status = 0;
        item_ev(&mut ctl, &mut f, event::ITEM_PICKED_UP, P1, c);
        assert_eq!(ctl.record(9).unwrap().status, 6);
    }
    // `tr1 ` with 10.3 set leaves the status.
    item_ev(&mut ctl, &mut f, event::ITEM_PICKED_UP, P2, b"tr1 ");
    assert_eq!(ctl.record(9).unwrap().status, 6);
    // Another code: flags := 0 only, nothing sent.
    ctl.record_mut(9).unwrap().flags = 3;
    f.sent.clear();
    item_ev(&mut ctl, &mut f, event::ITEM_PICKED_UP, P1, b"hst ");
    assert_eq!(ctl.record(9).unwrap().flags, 0);
    assert!(f.sent.is_empty());
    // F: 10.0 set without 10.13 / 10.14 → nothing sent.
    set(&mut f, P1, 10, 0);
    item_ev(&mut ctl, &mut f, event::ITEM_PICKED_UP, P1, b"vip ");
    assert!(f.sent.is_empty());
    // Drop clears the told bit of the code.
    for (c, b) in [(b"vip ", 4), (b"box ", 6), (b"msf ", 5)] {
        set(&mut f, P1, 10, b);
        item_ev(&mut ctl, &mut f, event::ITEM_DROPPED, P1, c);
        assert!(!f.flags(P1).get(10, b));
    }
    set(&mut f, P1, 10, 3);
    item_ev(&mut ctl, &mut f, event::ITEM_DROPPED, P1, b"tr1 ");
    assert!(f.flags(P1).get(10, 3));
    // Event 8 is a bare `ret` (handled, nothing logged).
    f.log.clear();
    ev(
        &mut ctl,
        &mut f,
        EventArgs {
            event: event::MONSTER_KILLED,
            target: Some(ITEM),
            player: Some(P1),
            ..EventArgs::default()
        },
    );
    assert!(f.log.is_empty());
}

// ------------------------------------------------------------ §4.9

// Covers: specs/world/quests-act2.md §4.9
#[test]
fn staff_assembly() {
    let (mut ctl, _) = control();
    let mut f = fake(false);
    x_mut(&mut ctl).staff_count = 1;
    x_mut(&mut ctl).amulet_count = 1;
    q2::staff_assembled(&mut ctl, &mut f, P1);
    let e = x(&ctl);
    assert!(e.assembled);
    assert_eq!(e.assembler, Some(1));
    assert_eq!((e.staff_count, e.amulet_count, e.hstaff_count), (0, 0, 1));
    // The 0x28 went out before 10.11 was set.
    assert_eq!(f.sent[0].1[0], 0x28);
    assert_eq!(f.sent[0].1[7 + 20..7 + 22], [0, 0]);
    assert_eq!(f.flags(P1).word(10), 1 << 11);
}

// ------------------------------------------------------------ §4.10

fn player_ev(ctl: &mut QuestControl, f: &mut Fake, e: u8, p: UnitId) {
    ev(
        ctl,
        f,
        EventArgs {
            event: e,
            target: Some(p),
            player: Some(p),
            ..EventArgs::default()
        },
    );
}

// Covers: specs/world/quests-act2.md §4.10, §4.2
#[test]
fn start_and_join_counts() {
    let (mut ctl, _) = control();
    let mut f = fake(true);
    give(&mut f, P1, &[b"msf ", b"vip ", b"tr1 "]);
    for b in [4, 5, 6] {
        set(&mut f, P1, 10, b);
    }
    // Chain 13 not-intro with the lair closed: clears 10.6 (no `box `).
    ctl.record_mut(13).unwrap().not_intro = true;
    player_ev(&mut ctl, &mut f, event::PLAYER_STARTED_GAME, P1);
    let e = x(&ctl);
    assert_eq!(
        (e.staff_count, e.amulet_count, e.cube_count, e.hstaff_count),
        (1, 1, 0, 0)
    );
    assert_eq!(f.flags(P1).word(10), 1 << 4 | 1 << 5);
    // The lair open, an `hst ` held, or 10.0 keeps the bits.
    for k in 0..3 {
        let (mut ctl, _) = control();
        let mut f = fake(false);
        set(&mut f, P1, 10, 4);
        match k {
            0 => ctl.record_mut(13).unwrap().extra.a2.q6.lair_open = true,
            1 => give(&mut f, P1, &[b"hst "]),
            _ => set(&mut f, P1, 10, 0),
        }
        player_ev(&mut ctl, &mut f, event::PLAYER_STARTED_GAME, P1);
        assert!(f.flags(P1).get(10, 4), "case {k}");
    }
    // Chain 13 intro: kept too.
    let (mut ctl, _) = control();
    let mut f = fake(false);
    ctl.record_mut(13).unwrap().not_intro = false;
    set(&mut f, P1, 10, 4);
    player_ev(&mut ctl, &mut f, event::PLAYER_STARTED_GAME, P1);
    assert!(f.flags(P1).get(10, 4));
    // Event 14: P2 brings the cube back; the staff is no longer missing.
    let (mut ctl, _) = control();
    let mut f = fake(true);
    {
        let e = x_mut(&mut ctl);
        (e.staff_count, e.amulet_count, e.missing_reported) = (1, 1, true);
    }
    ctl.record_mut(13).unwrap().extra.a2.q6.missing = true;
    give(&mut f, P2, &[b"box "]);
    player_ev(&mut ctl, &mut f, event::PLAYER_JOINED_GAME, P2);
    assert_eq!(x(&ctl).cube_count, 1);
    assert!(!ctl.record(13).unwrap().extra.a2.q6.missing);
    // With the staff items removed (+0x0E) it stays missing.
    let (mut ctl, _) = control();
    {
        let e = x_mut(&mut ctl);
        (e.hstaff_count, e.missing_reported) = (0, true);
    }
    let r13 = ctl.record_mut(13).unwrap();
    (r13.extra.a2.q6.missing, r13.extra.a2.q6.staff_removed) = (true, true);
    let mut f = fake(true);
    give(&mut f, P2, &[b"hst "]);
    player_ev(&mut ctl, &mut f, event::PLAYER_JOINED_GAME, P2);
    assert_eq!(x(&ctl).hstaff_count, 1);
    assert!(ctl.record(13).unwrap().extra.a2.q6.missing);
}

// Covers: specs/world/quests-act2.md §4.10
#[test]
fn leaving_with_pieces() {
    for (game_type, want) in [(3, 8), (0, 9)] {
        let (mut ctl, _) = control();
        let mut f = fake(false);
        f.game_type = game_type;
        {
            let e = x_mut(&mut ctl);
            (e.cube_count, e.cube_dropped, e.amulet_count) = (1, true, 1);
        }
        let leave = |ctl: &mut QuestControl, f: &mut Fake, code: &[u8; 4]| {
            f.item_codes.insert(ITEM, *code);
            ev(
                ctl,
                f,
                EventArgs {
                    event: event::PLAYER_DROPPED_WITH_QUEST_ITEM,
                    target: Some(ITEM),
                    player: Some(P1),
                    ..EventArgs::default()
                },
            );
        };
        // The amulet leaves: count 0 but the altar stands → nothing.
        leave(&mut ctl, &mut f, b"vip ");
        assert_eq!(x(&ctl).amulet_count, 0);
        assert!(!x(&ctl).missing_reported);
        // The cube leaves: missing.
        leave(&mut ctl, &mut f, b"box ");
        assert_eq!(x(&ctl).cube_count, 0);
        assert!(x(&ctl).missing_reported);
        let q6 = &ctl.record(13).unwrap().extra.a2.q6;
        assert!(q6.missing);
        assert_eq!(q6.missing_status, want);
        // Reported once.
        ctl.record_mut(13).unwrap().extra.a2.q6.missing = false;
        leave(&mut ctl, &mut f, b"msf ");
        assert_eq!(x(&ctl).staff_count, -1);
        assert!(!ctl.record(13).unwrap().extra.a2.q6.missing);
    }
    // Chain 13 intro: +0x2B still set, chain 13 untouched; an `hst `
    // in the game blocks it.
    let (mut ctl, _) = control();
    let mut f = fake(false);
    ctl.record_mut(13).unwrap().not_intro = false;
    x_mut(&mut ctl).staff_dropped = true;
    x_mut(&mut ctl).staff_count = 1;
    x_mut(&mut ctl).hstaff_count = 2;
    f.item_codes.insert(ITEM, *b"hst ");
    let args = EventArgs {
        event: event::PLAYER_DROPPED_WITH_QUEST_ITEM,
        target: Some(ITEM),
        player: Some(P1),
        ..EventArgs::default()
    };
    ev(&mut ctl, &mut f, args);
    assert!(!x(&ctl).missing_reported);
    f.item_codes.insert(ITEM, *b"msf ");
    ev(&mut ctl, &mut f, args);
    assert!(!x(&ctl).missing_reported); // hst count still 1
    f.item_codes.insert(ITEM, *b"hst ");
    ev(&mut ctl, &mut f, args);
    assert!(x(&ctl).missing_reported);
    assert!(!ctl.record(13).unwrap().extra.a2.q6.missing);
}

// Covers: specs/world/quests-act2.md §4.10, §1.1
// Covers: specs/world/quests-act2-2.md §1 r2
#[test]
fn leaving_town_and_game() {
    let (mut ctl, _) = control();
    let mut f = fake(true);
    ctl.record_mut(9).unwrap().guids.add(1);
    ctl.record_mut(9).unwrap().guids.add(2);
    // Event 10's `0x00545530` needs 10.0 and 10.1 (act2-2 §1 item 2).
    set(&mut f, P2, 10, 0);
    set(&mut f, P2, 10, 1);
    let lvl = |a, b| EventArgs {
        event: event::CHANGED_LEVEL,
        target: Some(P1),
        player: Some(P1),
        a,
        b,
    };
    ev(&mut ctl, &mut f, lvl(41, 40));
    assert_eq!(ctl.record(9).unwrap().guids.0, [1, 2]);
    ev(&mut ctl, &mut f, lvl(40, 41));
    assert_eq!(ctl.record(9).unwrap().guids.0, [2]);
    player_ev(&mut ctl, &mut f, event::PLAYER_LEAVES_GAME, P2);
    assert!(ctl.record(9).unwrap().guids.0.is_empty());
    assert!(f.log.is_empty());
}

// Covers: specs/world/quests-act2.md §4.1
#[test]
fn items_and_the_record_that_stays_on() {
    assert_eq!(
        [q2::SCROLL, q2::STAFF, q2::AMULET, q2::CUBE, q2::HSTAFF],
        [*b"tr1 ", *b"msf ", *b"vip ", *b"box ", *b"hst "]
    );
    assert_eq!((q2::CHAIN, q2::SLOT), (9, 10));
    // no_set_state = 1: a first player who finished the quest before
    // (10.12, 10.15) does not switch chain 9 off (`quests.md` §3).
    let (mut ctl, _) = control();
    let row = ctl.rows.iter().find(|r| r.chain == 9).unwrap();
    assert_eq!(row.no_set_state, Some(true));
    let mut f = fake(false);
    f.p(P1).quests.flags[0].set(10, bit::REWARD_GRANTED);
    f.p(P1).quests.flags[0].set(10, bit::COMPLETED_BEFORE);
    ctl.player_enters(&mut f, P1, 0).unwrap();
    let r = ctl.record(9).unwrap();
    assert!(r.not_intro && r.active);
    assert!(!ctl.game.get(10, bit::COMPLETED_BEFORE));
}
