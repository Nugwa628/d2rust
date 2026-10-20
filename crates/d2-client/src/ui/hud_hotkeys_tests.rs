use super::*;
use crate::bridge::skills::SkillEntry;

fn list(entries: &[(u16, u32, i32)]) -> SkillList {
    SkillList {
        entries: entries
            .iter()
            .map(|&(skill, owner, charges)| SkillEntry {
                skill,
                owner,
                charges,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn rec(x: i32, y: i32, skill: i32, owner: u32) -> Record {
    Record { x, y, skill, owner }
}

fn bytes(v: &[ClientIntent]) -> Vec<Vec<u8>> {
    v.iter().map(|i| i.0.clone()).collect()
}

const UNBIND0: [u8; 9] = [0x51, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];

#[test]
fn records_pad_to_sixty() {
    let icons = [SelectIcon {
        skill: 2,
        at: (672, 514),
        k: 0,
        owner: NATIVE,
        quantity: 0,
    }];
    let r = records(&icons);
    assert_eq!(r.len(), RECORDS);
    assert_eq!(r[0], rec(672, 514, 2, NATIVE));
    assert_eq!(r[1], rec(0, 0, 0, NATIVE));
}

#[test]
fn click_sends_select_and_closes() {
    // `ui-skill-hotkey-use-ama`: the right panel's click on skill 0 is
    // 3c 00 00 00 00 ff ff ff ff.
    let l = list(&[(0, NATIVE, 0)]);
    let recs = records(&[]);
    let mut recs = recs;
    recs[0] = rec(672, 514, 0, NATIVE);
    let mut left = false;
    let c = click(&recs, &l, &mut left, (696, 490), 600);
    assert!(c.close);
    assert_eq!(
        bytes(&c.intents),
        vec![vec![0x3C, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]]
    );
}

#[test]
fn shared_edge_sends_twice_left_then_right() {
    let l = list(&[(0, NATIVE, 0), (2, NATIVE, 0)]);
    let mut recs = records(&[]);
    recs[0] = rec(80, 514, 0, NATIVE);
    recs[1] = rec(128, 514, 2, NATIVE);
    let mut left = true;
    let c = click(&recs, &l, &mut left, (128, 500), 600);
    assert!(!left);
    let b = bytes(&c.intents);
    assert_eq!(b.len(), 2);
    assert_eq!(&b[0][1..5], &[0, 0, 0, 0x80]);
    assert_eq!(&b[1][1..5], &[2, 0, 0, 0]);
}

#[test]
fn spent_item_closes_without_send() {
    let l = list(&[(0, NATIVE, 0), (9, 0x55, 0)]);
    let mut recs = records(&[]);
    recs[0] = rec(80, 514, 9, 0x55);
    recs[1] = rec(80, 514, 0, NATIVE);
    let mut left = true;
    let c = click(&recs, &l, &mut left, (90, 500), 600);
    assert!(c.close);
    assert!(c.intents.is_empty());
    assert!(left);
}

#[test]
fn empty_click_closes_only_above_the_strip() {
    let l = list(&[]);
    let recs = records(&[]);
    let mut left = true;
    // (0, 0) is over every unused record but none has an entry.
    assert!(click(&recs, &l, &mut left, (300, 551), 600).close);
    assert!(!click(&recs, &l, &mut left, (300, 552), 600).close);
}

#[test]
fn assign_binds_unbinds_and_clears_duplicates() {
    let mut keys = Hotkeys::default();
    let mut recs = records(&[]);
    recs[0] = rec(672, 514, 0, NATIVE);
    // First bind: `ui-skill-hotkey-assign-ama`'s 51 00 00 00 00 ff ff ff ff.
    let out = assign(&mut keys, 0, &recs, false, (696, 490));
    assert_eq!(
        bytes(&out),
        vec![vec![0x51, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]]
    );
    // The same skill on slot 3: slot 0 is cleared.
    let out = assign(&mut keys, 3, &recs, false, (696, 490));
    assert_eq!(
        bytes(&out),
        vec![
            vec![0x51, 0, 0, 3, 0, 0xFF, 0xFF, 0xFF, 0xFF],
            UNBIND0.to_vec()
        ]
    );
    assert_eq!(keys.slots[0], Hotkey::NONE);
    // Rebinding slot 3 to another skill unbinds it first.
    recs[0] = rec(672, 514, 2, NATIVE);
    let out = assign(&mut keys, 3, &recs, true, (696, 490));
    assert_eq!(
        bytes(&out)[0],
        vec![0x51, 0xFF, 0xFF, 3, 0, 0xFF, 0xFF, 0xFF, 0xFF]
    );
    assert_eq!(
        bytes(&out)[1],
        vec![0x51, 2, 0x80, 3, 0, 0xFF, 0xFF, 0xFF, 0xFF]
    );
    // Nothing under the mouse: nothing changes.
    assert!(assign(&mut keys, 5, &recs, true, (400, 100)).is_empty());
}

#[test]
fn use_needs_an_entry_and_charges() {
    let l = list(&[(0, NATIVE, 0), (9, 0x55, 0)]);
    let mut keys = Hotkeys::default();
    assert_eq!(use_slot(&mut keys, 0, &l), None);
    keys.slots[4] = Hotkey {
        skill: 0,
        left: false,
        owner: NATIVE,
    };
    keys.slots[5] = Hotkey {
        skill: 9,
        left: false,
        owner: 0x55,
    };
    let u = use_slot(&mut keys, 4, &l).unwrap();
    assert_eq!(u.0, vec![0x3C, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(keys.last, 4);
    assert_eq!(use_slot(&mut keys, 5, &l), None);
    assert_eq!(keys.last, 4);
}

#[test]
fn wheel_skips_left_and_unusable_slots() {
    let l = list(&[(0, NATIVE, 0), (2, NATIVE, 0), (3, NATIVE, 0)]);
    let mut keys = Hotkeys::default();
    let hk = |skill, left| Hotkey {
        skill,
        left,
        owner: NATIVE,
    };
    keys.slots[1] = hk(2, true);
    keys.slots[2] = hk(2, false);
    keys.slots[3] = hk(7, false);
    keys.slots[15] = hk(3, false);
    // +1 from 0: slot 1 is left, slot 2 counts.
    let u = wheel(&mut keys, 1, &l).unwrap();
    assert_eq!(u.0[1], 2);
    assert_eq!(keys.last, 2);
    // +2 from 2: slot 3 has no entry, 15 then 2 (wrap).
    wheel(&mut keys, 2, &l).unwrap();
    assert_eq!(keys.last, 2);
    // −1 from 2: back over 1 (left) and 0 (empty) to 15.
    wheel(&mut keys, -1, &l).unwrap();
    assert_eq!(keys.last, 15);
    assert_eq!(wheel(&mut keys, 0, &l), None);
}

#[test]
fn server_assign_drops_out_of_range() {
    let mut keys = Hotkeys::default();
    server_assign(&mut keys, 2, 6, true, NATIVE);
    server_assign(&mut keys, 16, 6, true, NATIVE);
    assert_eq!(
        keys.slots[2],
        Hotkey {
            skill: 6,
            left: true,
            owner: NATIVE
        }
    );
    assert_eq!(keys.slots.iter().filter(|s| s.skill != -1).count(), 1);
}

#[test]
fn slot_commands_and_labels() {
    assert_eq!(slot_cmd(0), 14);
    assert_eq!(slot_cmd(7), 21);
    assert_eq!(slot_cmd(8), 46);
    assert_eq!(slot_cmd(15), 53);
    let mut keys = Hotkeys::default();
    keys.slots[0] = Hotkey {
        skill: 0,
        left: false,
        owner: NATIVE,
    };
    keys.slots[9] = Hotkey {
        skill: 0,
        left: true,
        owner: NATIVE,
    };
    let r = rec(672, 514, 0, NATIVE);
    assert_eq!(labels_of(&keys, &r, false), vec![0]);
    assert_eq!(labels_of(&keys, &r, true), vec![9]);
}

#[test]
fn label_and_count_places() {
    // `ui-skill-hotkey-assign-ama` rows 232–236: "F1" at (x + 34, y − 35).
    assert_eq!(label_at((672, 514), 13), (706, 479));
    assert_eq!(label_at((635, 600), 9), (669, 565));
    assert_eq!(label_at((0, 100), 20), (27, 65));
    assert_eq!(count_text(0, (10, 20)), None);
    assert_eq!(
        count_text(12, (10, 20)),
        Some(("12".encode_utf16().collect(), (14, 19)))
    );
}

#[test]
fn fit_cuts_to_forty_pixels() {
    let w = |t: &[u16]| Some(t.len() as i32 * 7);
    let t: Vec<u16> = "ScrollLock".encode_utf16().collect();
    assert_eq!(
        fit(&t, &w).unwrap(),
        "Scrol".encode_utf16().collect::<Vec<_>>()
    );
    let t: Vec<u16> = "F1".encode_utf16().collect();
    assert_eq!(fit(&t, &w).unwrap(), t);
    assert_eq!(fit(&t, &|_| None), None);
}
