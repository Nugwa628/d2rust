//! Test vectors of the skill messages (0x21–0x23, 0x94), the UI messages
//! (0x5D, 0x63, 0x77), 0x2C and 0x53. "A" and "B" are two recorded
//! packet traces; the rest synthetic.

use super::super::output::Output;
use super::super::skills::NATIVE;
use super::super::world::{SkillRow, UnitKey, MONSTER, PLAYER};
use super::support::{hex, Model};

const P1: UnitKey = UnitKey::new(PLAYER, 1);

/// 0x59 for player 1 (class 1) at (x, y).
fn assign_player(x: u16, y: u16) -> Vec<u8> {
    let mut b = hex("59 01 00 00 00 01 77 65 72 77 65 72");
    b.resize(0x16, 0);
    b.extend_from_slice(&x.to_le_bytes());
    b.extend_from_slice(&y.to_le_bytes());
    b
}

/// A model with `n` skills rows (no passive state) and player 1.
fn with_player(n: usize) -> Model {
    let mut m = Model::default();
    m.inputs.tables.skills = vec![SkillRow::default(); n];
    m.recv(&assign_player(0, 0));
    m
}

fn skills_of(m: &Model) -> Vec<(u16, i32, u32)> {
    m.unit(P1)
        .skills
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|e| (e.skill, e.base, e.owner))
        .collect()
}

const A_273: &str = "94 0a 01 00 00 00 00 00 01 02 00 01 01 00 01 d9 00 01 da 00 01 db 00 01 \
                     dc 00 01 04 00 01 05 00 01 03 00 01";

#[test]
fn base_skill_levels_a273() {
    let mut m = with_player(221);
    m.hex(A_273);
    let want: Vec<(u16, i32, u32)> = [0, 2, 1, 217, 218, 219, 220, 4, 5, 3]
        .iter()
        .map(|&s| (s, 1, NATIVE))
        .collect();
    assert_eq!(skills_of(&m), want);
    assert!(m.log.rejected.is_empty());
    // No skill-tree flag.
    assert_eq!(m.w.skill_tree_flag, None);
    // An unknown GUID: no change.
    let before = m.w.clone();
    m.hex("94 01 02 00 00 00 24 00 01");
    assert_eq!(m.w, before);
}

#[test]
fn update_item_skill_quantities() {
    // B seq 227: 8 entries (the entry list is synthetic), then 0x22.
    let mut m = with_player(221);
    m.hex(
        "94 08 01 00 00 00 00 00 01 02 00 01 01 00 01 d9 00 01 da 00 01 db 00 01 dc 00 01 03 00 01",
    );
    assert_eq!(skills_of(&m).len(), 8);
    m.hex("22 00 19 01 00 00 00 db 00 01 41 00");
    let q = |m: &Model| {
        let l = m.unit(P1).skills.as_ref().unwrap();
        l.entries[l.native(219).unwrap()].quantity
    };
    assert_eq!(q(&m), 1);
    // A seq 273.
    m.hex("22 00 19 01 00 00 00 db 00 00 55 00");
    assert_eq!(q(&m), 0);
    // Byte @11 = 1: no change.
    m.hex("22 00 19 01 00 00 00 db 00 07 55 01");
    assert_eq!(q(&m), 0);
    assert!(m.log.rejected.is_empty());
    // A skill the player lacks: fatal 0xAD6.
    m.hex("22 00 19 01 00 00 00 24 00 01 41 00");
    assert_eq!(m.rejected(), [(0x22, "fatal assert 0xAD6".to_owned())]);
}

#[test]
fn set_skill_left_and_right() {
    let mut m = with_player(221);
    m.hex(A_273);
    // A seq 274.
    m.hex("23 00 01 00 00 00 01 00 00 ff ff ff ff")
        .hex("23 00 01 00 00 00 00 00 00 ff ff ff ff");
    let l = m.unit(P1).skills.clone().unwrap();
    assert_eq!(l.left_entry().map(|e| e.skill), Some(0));
    assert_eq!(l.right_entry().map(|e| e.skill), Some(0));
    // B seq 227 (list state synthetic): skill 36 without an entry.
    m.hex("23 00 01 00 00 00 00 24 00 ff ff ff ff");
    assert_eq!(m.unit(P1).skills.as_ref().unwrap().right, l.right);
    assert!(m.log.rejected.is_empty());
    // A skill outside the table: fatal 0x668.
    m.hex("23 00 01 00 00 00 00 dd 00 ff ff ff ff");
    assert_eq!(m.rejected(), [(0x23, "fatal assert 0x668".to_owned())]);
}

#[test]
fn update_item_oskill_b46370_b61126() {
    let mut m = with_player(221);
    // B seq 46370: no skill 36 → added, then base 0; the flag cleared.
    m.hex("21 00 00 01 00 00 00 24 00 00 01 05");
    assert_eq!(skills_of(&m), [(36, 0, NATIVE)]);
    assert_eq!(m.w.skill_tree_flag, Some(0));
    // B seq 61126.
    m.hex("21 00 00 01 00 00 00 24 00 01 01 05");
    assert_eq!(skills_of(&m), [(36, 1, NATIVE)]);
    // Synthetic: level 0 with the remove flag.
    m.hex("21 00 01 01 00 00 00 24 00 00 00 00");
    assert!(skills_of(&m).is_empty());
    assert!(m.log.rejected.is_empty());
}

#[test]
fn players_and_monsters_get_a_list_at_creation() {
    let m = with_player(0);
    assert_eq!(m.unit(P1).skills.as_ref().map(|l| l.entries.len()), Some(0));
    // A unit without a list: the messages do nothing.
    let mut m = Model::default();
    m.inputs.tables.skills = vec![SkillRow::default(); 40];
    m.put(P1);
    m.hex("21 00 00 01 00 00 00 24 00 01 01 05")
        .hex("94 01 01 00 00 00 24 00 01")
        .hex("23 00 01 00 00 00 00 24 00 ff ff ff ff");
    assert_eq!(m.unit(P1).skills, None);
    assert!(m.log.rejected.is_empty());
}

#[test]
fn a_passive_skill_is_rejected_as_pending() {
    let mut m = with_player(40);
    m.inputs.tables.skills[36].passivestate = 30;
    m.hex("21 00 00 01 00 00 00 24 00 01 01 05");
    assert_eq!(skills_of(&m), [(36, 1, NATIVE)]);
    assert_eq!(m.log.rejected.len(), 1);
    assert!(m.rejected()[0]
        .1
        .starts_with("TODO(spec: client/msg-skills.md §2 r4"));
    // 0x94 assigns every entry, then reports the pending part.
    m.hex("94 02 01 00 00 00 24 00 02 25 00 01");
    assert_eq!(skills_of(&m), [(36, 2, NATIVE), (37, 1, NATIVE)]);
    assert_eq!(m.log.rejected.len(), 2);
}

// --------------------------------------------------------------- 0x5D

/// The outputs of the model so far.
fn outs(m: &Model) -> Vec<Output> {
    m.out.clone()
}

#[test]
fn quest_status_output_rows() {
    let mut m = Model::default();
    // A seq 129118.
    m.hex("5d 01 00 01 00 00");
    assert_eq!(
        outs(&m),
        [Output::QuestUi {
            chain: 1,
            flags: 0,
            status: 1,
            extra: 0
        }]
    );
    assert_eq!(m.w, Model::default().w, "no model change");
    m.out.clear();
    m.hex("5d 08 02 00 00 00").hex("5d 21 10 00 05 00");
    assert_eq!(
        outs(&m),
        [
            Output::QuestUi {
                chain: 8,
                flags: 2,
                status: 0,
                extra: 0
            },
            Output::QuestUi {
                chain: 0x21,
                flags: 0x10,
                status: 0,
                extra: 5
            },
        ]
    );
    m.out.clear();
    // f = 3, c = 8: the bit-0 table has no 8 (edge case 1): a "nothing"
    // row, not an output row (step 5).
    m.hex("5d 08 03 00 00 00");
    assert!(outs(&m).is_empty());
    // The extra word is read signed.
    m.hex("5d 05 20 03 ff ff");
    assert_eq!(
        outs(&m),
        [Output::QuestUi {
            chain: 5,
            flags: 0x20,
            status: 3,
            extra: -1
        }]
    );
    assert!(m.log.rejected.is_empty());
}

#[test]
fn quest_status_model_rows() {
    let mut m = Model::default();
    m.hex("5d 17 01 00 00 00");
    assert!(m.w.exit_requested && outs(&m).is_empty());
    let mut m = Model::default();
    let (a, b, c) = (
        UnitKey::new(MONSTER, 5),
        UnitKey::new(MONSTER, 6),
        UnitKey::new(MONSTER, 9),
    );
    m.put(a).class = 146;
    m.put(b).class = 147;
    m.put(c).class = 527;
    m.put(UnitKey::new(PLAYER, 146)).class = 146;
    m.hex("5d 04 01 00 00 00");
    assert!(m.unit(a).quest_untargetable && !m.unit(b).quest_untargetable);
    assert!(!m.unit(c).quest_untargetable);
    assert!(!m.unit(UnitKey::new(PLAYER, 146)).quest_untargetable);
    m.hex("5d 21 20 00 00 00");
    assert!(m.unit(c).quest_untargetable);
    assert!(outs(&m).is_empty());
    assert!(m.log.rejected.is_empty());
}

#[test]
fn quest_status_eclipse() {
    // No client act: the pending flag.
    let mut m = Model::default();
    m.hex("5d 0a 01 00 00 00");
    assert!(m.w.eclipse_pending && outs(&m).is_empty());
    // With a client act the setter runs now with index 5, ticks 0,
    // eclipse 1: the period reset takes type and
    // ticks from eclipse entry 5 (type 2, 240 × speed = 30,720).
    let mut m = Model::default();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.hex("5d 0a 01 00 00 00");
    assert!(m.log.rejected.is_empty());
    let e = m.w.environment.unwrap();
    assert_eq!((e.index, e.kind, e.ticks, e.eclipse), (5, 2, 30_720, true));
    assert!(!m.w.eclipse_pending && outs(&m).is_empty());
    // A pending eclipse turns into the setter call at the act-2 load.
    let mut m = Model::default();
    m.hex("5d 0a 01 00 00 00");
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    assert!(m.log.rejected.is_empty(), "act 1: nothing");
    m.hex("03 01 c4 88 38 10 01 00 61 d1 e0 9f");
    assert!(m.log.rejected.is_empty());
    let e = m.w.environment.unwrap();
    assert_eq!((e.index, e.kind, e.ticks, e.eclipse), (5, 2, 30_720, true));
}

#[test]
fn waypoint_menu_b7852() {
    let mut m = Model::default();
    m.hex("63 0a 00 00 00 02 01 03 00 00 00 00 00 00 00 00 00 00 00 00 00");
    let mut record = [0u8; 16];
    record[..4].copy_from_slice(&[2, 1, 3, 0]);
    assert_eq!(outs(&m), [Output::WaypointMenu { guid: 0x0A, record }]);
    assert_eq!(m.w, Model::default().w, "no model change");
}

#[test]
fn trade_action_a104179() {
    let mut m = Model::default();
    m.hex("77 10").hex("77 16");
    assert_eq!(
        outs(&m),
        [
            Output::TradeAction {
                code: 0x10,
                dead_or_absent: true
            },
            Output::TradeAction {
                code: 0x16,
                dead_or_absent: true
            }
        ]
    );
    assert_eq!(m.w, Model::default().w);
}

// --------------------------------------------------------------- 0x2C

#[test]
fn play_sound_captures_key_and_class() {
    let mut m = Model::default();
    let npc = UnitKey::new(MONSTER, 0x26);
    m.put(npc).class = 0x93;
    // A seq 293508; then a unit not in S: nothing.
    m.hex("2c 01 26 00 00 00 12 00")
        .hex("2c 01 27 00 00 00 12 00");
    assert_eq!(
        outs(&m),
        [Output::ServerSound {
            unit: npc,
            class: 0x93,
            at: None,
            event: 18
        }]
    );
    assert!(m.log.rejected.is_empty());
}

#[test]
fn outputs_keep_message_order_and_capture_at_receive() {
    let mut m = Model::default();
    let npc = UnitKey::new(MONSTER, 0x26);
    let u = m.put(npc);
    u.class = 0x93;
    u.position = Some((5000, 4100));
    // One chunk: 0x2C on (1, 0x26), 0x77 0x10, then 0x0A removing the
    // unit: the sound keeps the class and position read at receive, and
    // the free appends its `UnitFreed` after them.
    let chunk = [
        hex("2c 01 26 00 00 00 12 00"),
        hex("77 10"),
        hex("0a 01 26 00 00 00"),
    ]
    .concat();
    m.recv(&chunk);
    assert!(m.w.units.is_empty());
    assert_eq!(
        outs(&m),
        [
            Output::ServerSound {
                unit: npc,
                class: 0x93,
                at: Some((5000, 4100)),
                event: 18
            },
            Output::TradeAction {
                code: 0x10,
                dead_or_absent: true
            },
            Output::UnitFreed {
                unit: npc,
                client_only: false
            }
        ]
    );
}

#[test]
fn every_model_unit_free_appends_one_unit_freed() {
    let mut m = Model::default();
    let a = UnitKey::new(MONSTER, 0x26);
    let b = UnitKey::new(MONSTER, 0x27);
    m.put(a);
    m.put(b);
    // A key not in S frees nothing; two frees keep their order.
    m.recv(
        &[
            hex("0a 01 30 00 00 00"),
            hex("0a 01 27 00 00 00"),
            hex("0a 01 26 00 00 00"),
        ]
        .concat(),
    );
    assert_eq!(
        outs(&m),
        [
            Output::UnitFreed {
                unit: b,
                client_only: false
            },
            Output::UnitFreed {
                unit: a,
                client_only: false
            }
        ]
    );
    assert!(m.w.freed.is_empty());
}

// --------------------------------------------------------------- 0x53

/// Act 1 loaded, player 1 placed at (x, y) and local.
fn placed(x: u16, y: u16) -> Model {
    let mut m = Model::default();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&assign_player(x, y)).hex("0b 00 01 00 00 00");
    m
}

#[test]
fn darkness_b228_b146616() {
    let mut m = placed(0x1241, 0x11C4);
    let env = m.w.environment.unwrap();
    assert_eq!((env.index, env.ticks, env.intensity), (2, 0, 128), "§9.1");
    // B seq 228 and seq 146616.
    m.hex("53 02 00 00 00 00 00 00 00 00");
    let e = m.w.environment.unwrap();
    assert_eq!((e.index, e.ticks), (2, 0));
    m.hex("53 02 00 00 00 80 08 00 00 00");
    let e = m.w.environment.unwrap();
    assert_eq!((e.index, e.ticks, e.eclipse), (2, 0x880, false));
    assert!(m.log.rejected.is_empty() && outs(&m).is_empty());
    // Ticks past speed × 360 → 0.
    m.hex("53 03 00 00 00 01 b4 00 00 00");
    assert_eq!(m.w.environment.unwrap().ticks, 0);
}

#[test]
fn darkness_needs_the_players_act() {
    // An unplaced player has no act: ignored.
    let mut m = Model::default();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.recv(&assign_player(0, 0)).hex("0b 00 01 00 00 00");
    let before = m.w.environment;
    m.hex("53 04 00 00 00 00 00 00 00 00");
    assert_eq!(m.w.environment, before);
    assert!(m.log.rejected.is_empty());
    // No local player: the base game reads through a null pointer.
    let mut m = Model::default();
    m.hex("03 00 c4 88 38 10 01 00 61 d1 e0 9f");
    m.hex("53 02 00 00 00 00 00 00 00 00");
    assert_eq!(m.log.rejected.len(), 1);
    // Fatal values, then the eclipse branch.
    let mut m = placed(0x1241, 0x11C4);
    m.hex("53 06 00 00 00 00 00 00 00 00")
        .hex("53 02 00 00 00 ff ff ff ff 00")
        .hex("53 05 00 00 00 00 00 00 00 01");
    let r = m.rejected();
    assert_eq!(r.len(), 2);
    assert!(r[0].1.contains("period index") && r[1].1.contains("negative ticks"));
    // The eclipse branch now runs: eclipse entry 5's type and
    // start × speed replace the received ticks 0.
    let e = m.w.environment.unwrap();
    assert_eq!((e.index, e.kind, e.ticks, e.eclipse), (5, 2, 30_720, true));
}

#[test]
fn darkness_object_day_refresh() {
    use super::super::world::{ObjectRow, OBJECT};
    let mut m = placed(0x1241, 0x11C4);
    m.inputs.tables.objects = vec![ObjectRow::default(); 40];
    m.inputs.tables.objects[39] = ObjectRow {
        env_effect: true,
        lit: [0, 19, 19, 0, 0, 0, 0, 0],
        rgb: (255, 236, 176),
        ..ObjectRow::default()
    };
    let fire = UnitKey::new(OBJECT, 0x20);
    let u = m.put(fire);
    u.class = 39;
    u.position = Some((0x1241, 0x11C4));
    let lights = |m: &Model| m.w.lights.len();
    let before = lights(&m);
    // Index 2: day (type 0) = the zero cache: nothing.
    m.hex("53 02 00 00 00 00 00 00 00 00");
    assert_eq!((m.unit(fire).mode, lights(&m)), (0, before));
    // Index 4: type 1 → mode 1, its light (radius 19 / 2).
    m.hex("53 04 00 00 00 00 00 00 00 00");
    assert_eq!(m.w.env_period_cache, 1);
    assert_eq!((m.unit(fire).mode, m.unit(fire).flag_2), (1, Some(false)));
    assert_eq!(lights(&m), before + 1);
    let (id, rec) = m.w.lights.iter().next().unwrap();
    assert_eq!((rec.owner_guid, m.w.lights.radius(id)), (0x20, Some(9)));
    // Back to day: mode 0, `Lit0` = 0 removes the light.
    m.hex("53 02 00 00 00 00 00 00 00 00");
    assert_eq!((m.unit(fire).mode, lights(&m)), (0, before));
    assert!(m.log.rejected.is_empty());
}

#[test]
fn the_local_players_skills_come_from_four_messages() {
    let mut m = with_player(221);
    // 0x94: native entries and levels (join).
    m.hex("94 02 01 00 00 00 00 00 01 db 00 02");
    // 0x21: a later change of a native level.
    m.hex("21 00 00 01 00 00 00 db 00 05 01 05");
    // 0x22: the quantity of a tome / scroll skill.
    m.hex("22 00 19 01 00 00 00 db 00 03 41 00");
    // 0x23: left and right.
    m.hex("23 00 01 00 00 00 01 db 00 ff ff ff ff");
    let l = m.unit(P1).skills.clone().unwrap();
    let e = l.entries[l.native(219).unwrap()];
    assert_eq!((e.base, e.quantity), (5, 3));
    assert_eq!(l.left_entry().map(|e| e.skill), Some(219));
    assert_eq!(l.right, None, "right unchanged");
    assert!(m.log.rejected.is_empty());
}

#[test]
fn skill_message_edge_cases() {
    let mut m = with_player(221);
    // 0x94 with level 0 creates the entry at base 0.
    m.hex("94 01 01 00 00 00 24 00 00");
    assert_eq!(skills_of(&m), [(36, 0, NATIVE)]);
    // 0x21 with level 0 and the remove flag deletes it.
    m.hex("21 00 01 01 00 00 00 24 00 00 00 00");
    assert!(skills_of(&m).is_empty());
    // 0x22 and 0x23 ignore the unit-type byte: players only.
    m.hex("94 01 01 00 00 00 db 00 01");
    m.hex("22 07 19 01 00 00 00 db 00 04 41 00");
    m.hex("23 05 01 00 00 00 01 db 00 ff ff ff ff");
    let l = m.unit(P1).skills.clone().unwrap();
    assert_eq!(l.entries[0].quantity, 4);
    assert_eq!(l.left_entry().map(|e| e.skill), Some(219));
    // The bytes the sender never writes are not read: 0x22 @2 / @10 and
    // 0x21 @10 / @11 vary without effect.
    let before = m.w.clone();
    m.hex("22 00 55 01 00 00 00 db 00 04 99 00");
    m.hex("21 00 00 01 00 00 00 db 00 01 77 66");
    assert_eq!(skills_of(&m), [(219, 1, NATIVE)]);
    assert_eq!(m.unit(P1).skills.as_ref().unwrap().entries[0].quantity, 4);
    assert_eq!(m.w, before, "the unread bytes change nothing");
    // A monster's add may exceed max_level (checked on the list: skills.rs).
}

#[test]
fn a_player_mode_request_removes_the_cast_light_not_the_player_light() {
    use crate::rules::lighting::records::LightKind;
    let mut m = with_player(1);
    // Player init: the player light (unit `+0x64`).
    assert_eq!(m.w.lights.len(), 1);
    // detaches the stat list's cast light only: none here.
    super::super::modes::remove_unit_light(&mut m.w, P1);
    assert_eq!(m.w.lights.len(), 1, "the player light stays");
    let cast =
        m.w.lights
            .create(None, (0, 0), LightKind::Plain, 1, 255, 255, 255, 255)
            .unwrap();
    m.w.cast_lights.insert(P1, cast);
    super::super::modes::remove_unit_light(&mut m.w, P1);
    assert_eq!(m.w.lights.len(), 1);
    assert!(m.w.lights.get(cast).is_none() && m.w.cast_lights.is_empty());
}
