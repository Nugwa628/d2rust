//! The save gaps (q-save-gaps): mouse skills of both weapon sets, the town
//! byte's act, the hireling's and the golem's items round-trip through a
//! written file field by field. D2RUST's own, unverified.

use d2_client::app::save_gaps::{apply_gaps, mouse_slots, select_mouse, Gaps};
use d2_formats::d2s::{
    self, Body, D2s, Golem, Header, Hireling, ItemEntry, ReadOptions, Slot, StatSave,
};
use d2_server::adapters::handlers::world::HirelingBlock;
use d2_sim::skills::list::{ListEntry, SkillList};

/// 32-bit stats; every item entry is 3 bytes.
struct Tables;

impl d2s::SaveTables for Tables {
    fn stat_save(&self, id: u16) -> Option<StatSave> {
        (id < 400).then_some(StatSave {
            bits: 32,
            param: 0,
            signed: true,
        })
    }
    fn item_entry_len(&self, _: &[u8]) -> Result<usize, String> {
        Ok(3)
    }
}

fn item(b: u8) -> ItemEntry {
    ItemEntry { bytes: vec![b; 3] }
}

fn base() -> D2s {
    let mut header = Header::default();
    header.set_name(b"Rolf").unwrap();
    header.class = 1;
    header.status = d2s::status::EXPANSION;
    header.towns = [0x80, 0, 0];
    header.hireling.seed = 77;
    // The weapon-swap pair of the loaded file.
    header.mouse[2] = Slot::encode(40, false, 0).unwrap();
    header.mouse[3] = Slot::encode(41, true, 0).unwrap();
    D2s {
        header,
        body: Some(Body {
            skills: vec![0; 30],
            hireling_items: Some(Some(vec![])),
            golem: Some(Golem {
                flag: 0,
                item: None,
            }),
            ..Body::default()
        }),
    }
}

fn entry(skill: u16, owner: i32) -> ListEntry {
    ListEntry {
        skill,
        base: 1,
        owner,
        ..ListEntry::default()
    }
}

/// Mouse skills, the act, the hireling's items and the golem's item are
/// written and read back field by field.
#[test]
fn gaps_round_trip_through_a_written_file() {
    let gaps = Gaps {
        mouse: Some([
            Slot::encode(36, true, 0).unwrap(),
            Slot::encode(59, false, 2).unwrap(),
        ]),
        town: Some((1, 3)),
        hotkeys: None,
        hireling_items: Some(vec![item(1), item(2)]),
        golem: Some(Some(item(7))),
        swap: Some((
            [
                Slot::encode(40, true, 0).unwrap(),
                Slot::encode(43, false, 1).unwrap(),
            ],
            true,
        )),
        hireling: Some(HirelingBlock {
            dead: true,
            seed: 0xAB12,
            name_index: 5,
            id: 3,
            experience: 1234,
        }),
        status: Some(0x0520),
    };
    let mut save = base();
    apply_gaps(&mut save, &gaps);
    let bytes = d2s::write(&save, &Tables).unwrap();
    let opts = ReadOptions {
        expansion: true,
        game: None,
    };
    let back = d2s::read(&bytes, &opts, &Tables).unwrap();
    let h = &back.header;
    assert_eq!(h.mouse[0], gaps.mouse.unwrap()[0]);
    assert_eq!(h.mouse[1], gaps.mouse.unwrap()[1]);
    let swap = gaps.swap.unwrap().0;
    assert_eq!((h.mouse[2], h.mouse[3]), (swap[0], swap[1]));
    assert_eq!(h.weapon_switch, 1);
    assert_eq!(
        (h.hireling.flags, h.hireling.seed, h.hireling.name_index),
        (Hireling::DEAD, 0xAB12, 5)
    );
    assert_eq!((h.hireling.id, h.hireling.experience), (3, 1234));
    assert_eq!(d2s::status::progression(h.status), 5);
    assert!(h.status & d2s::status::EXPANSION != 0);
    // Nightmare's byte: act 3 | 0x80; the others are zero.
    assert_eq!(h.towns, [0, 0x83, 0]);
    let b = back.body.unwrap();
    assert_eq!(b.hireling_items, Some(Some(vec![item(1), item(2)])));
    assert_eq!(
        b.golem,
        Some(Golem {
            flag: 1,
            item: Some(item(7))
        })
    );
}

/// Nothing read from the game leaves the loaded values as they were.
#[test]
fn no_gaps_keep_the_loaded_values() {
    let mut save = base();
    save.header.mouse[0] = Slot::encode(36, true, 0).unwrap();
    let before = save.clone();
    apply_gaps(&mut save, &Gaps::default());
    assert_eq!(save, before);
}

/// A golem without an item clears the section; a classic save has none.
#[test]
fn a_golem_without_an_item_writes_flag_zero() {
    let mut save = base();
    save.body.as_mut().unwrap().golem = Some(Golem {
        flag: 1,
        item: Some(item(9)),
    });
    apply_gaps(
        &mut save,
        &Gaps {
            golem: Some(None),
            ..Gaps::default()
        },
    );
    assert_eq!(
        save.body.unwrap().golem,
        Some(Golem {
            flag: 0,
            item: None
        })
    );
}

/// The mouse skills of a list encode with the 1-based position of the
/// owner item and select again on the loaded list.
#[test]
fn mouse_skills_select_with_their_item() {
    let guids = [500u32, 501, 502];
    let mut list = SkillList {
        entries: vec![entry(0, -1), entry(36, -1), entry(36, 501), entry(59, -1)],
        left: Some(2),
        right: Some(3),
        ..SkillList::default()
    };
    let slots = mouse_slots(&list, &guids);
    // No 0x8000 left flag on a mouse word.
    assert_eq!(slots[0], Slot::encode(36, false, 2).unwrap());
    assert_eq!(slots[0].code, 36);
    assert_eq!(slots[1], Slot::encode(59, false, 0).unwrap());
    // The loaded list starts with nothing selected.
    let (l, r) = (list.left.take(), list.right.take());
    assert_eq!((l, r), (Some(2), Some(3)));
    select_mouse(&mut list, &slots, &guids);
    assert_eq!((list.left, list.right), (Some(2), Some(3)));
}

/// No left skill is the all-zero pair; an item past the list looks up the
/// owner GUID −1 (exact skill and owner, no class
/// fallback), here the native entry; skill 0 is never selected.
#[test]
fn absent_and_unknown_mouse_entries() {
    let list = SkillList {
        entries: vec![entry(0, -1), entry(36, -1)],
        left: None,
        right: Some(1),
        ..SkillList::default()
    };
    let slots = mouse_slots(&list, &[]);
    assert_eq!(slots[0], Slot::default());
    assert_eq!(slots[1], Slot::encode(36, false, 0).unwrap());
    let mut loaded = SkillList {
        entries: vec![entry(0, -1), entry(36, -1)],
        ..SkillList::default()
    };
    // Left skill 36 on item index 5, which the list does not have: owner
    // −1, the native entry is found by (skill, owner) exactly.
    select_mouse(
        &mut loaded,
        &[Slot::encode(36, true, 5).unwrap(), slots[1]],
        &[],
    );
    assert_eq!((loaded.left, loaded.right), (Some(1), Some(1)));
    // An entry that is not in the list (skill 40, owner GUID 77) leaves
    // the hand as it was: no fallback to the class entry of that skill.
    let mut kept = SkillList {
        entries: vec![entry(0, -1), entry(36, -1), entry(40, -1)],
        left: Some(1),
        right: Some(1),
        ..SkillList::default()
    };
    select_mouse(
        &mut kept,
        &[
            Slot::encode(40, false, 1).unwrap(),
            Slot::encode(5, false, 0).unwrap(),
        ],
        &[77],
    );
    assert_eq!(
        (kept.left, kept.right),
        (Some(1), Some(1)),
        "no entry (40, 77); skill 5"
    );
    // The all-zero pairs (skill 0, Attack) select nothing.
    let mut none = SkillList {
        entries: vec![entry(0, -1)],
        ..SkillList::default()
    };
    select_mouse(&mut none, &[Slot::default(), Slot::default()], &[]);
    assert_eq!((none.left, none.right), (None, None));
}

/// A hireling hired since the load has no header block yet, so its list
/// is not written (it would not read back).
#[test]
fn hireling_items_need_a_hireling_block() {
    let mut save = base();
    save.header.hireling = d2s::Hireling::default();
    save.body.as_mut().unwrap().hireling_items = Some(None);
    apply_gaps(
        &mut save,
        &Gaps {
            hireling_items: Some(vec![item(1)]),
            ..Gaps::default()
        },
    );
    assert_eq!(save.body.unwrap().hireling_items, Some(None));
}

/// The progression is never lowered: flags
/// with a lower progression leave the loaded bits.
#[test]
fn progression_is_never_lowered() {
    let mut save = base();
    save.header.status |= 7 << 8;
    apply_gaps(
        &mut save,
        &Gaps {
            status: Some(0x0120),
            ..Gaps::default()
        },
    );
    assert_eq!(d2s::status::progression(save.header.status), 7);
}

/// The swap pair of a list encodes with its owner item and the weapon
/// switch trades the pairs.
#[test]
fn weapon_switch_trades_the_mouse_pairs() {
    let mut list = SkillList {
        entries: vec![entry(36, -1), entry(37, -1), entry(38, 500)],
        left: Some(0),
        right: Some(2),
        swap_left: Some((37, -1)),
        swap_right: None,
        ..SkillList::default()
    };
    let saved = list.trade_swap_pairs();
    assert_eq!(saved, [(37, -1), (0, -1)], "unset reads as (0, −1)");
    assert!(list.weapon_switch);
    assert_eq!(
        (list.swap_left, list.swap_right),
        (Some((36, -1)), Some((38, 500)))
    );
    let swap = d2_client::app::save_gaps::swap_slots(&list, &[500]);
    assert_eq!(swap[0], Slot::encode(36, false, 0).unwrap());
    assert_eq!(swap[1], Slot::encode(38, false, 1).unwrap());
    list.trade_swap_pairs();
    assert!(!list.weapon_switch);
}

/// The bytes of a written mouse pair (test vectors): the left
/// skill 36 on a native entry is `24 00 00 00` at +0x78 (no 0x8000 flag),
/// the swap pair (36 left, 0 right) with the switch byte set is
/// `24 00 00 00` / `00 00 00 00` at +0x80 / +0x84 and `01 00 00 00` at +0x10.
#[test]
fn mouse_words_have_no_left_flag_in_the_file() {
    let list = SkillList {
        entries: vec![entry(0, -1), entry(36, -1)],
        left: Some(1),
        right: Some(0),
        swap_left: Some((36, -1)),
        swap_right: Some((0, -1)),
        weapon_switch: true,
        ..SkillList::default()
    };
    let mut save = base();
    let gaps = Gaps {
        mouse: Some(mouse_slots(&list, &[])),
        swap: Some((d2_client::app::save_gaps::swap_slots(&list, &[]), true)),
        ..Gaps::default()
    };
    apply_gaps(&mut save, &gaps);
    let b = save.header.to_bytes();
    assert_eq!(&b[0x78..0x7C], &[0x24, 0, 0, 0]);
    assert_eq!(&b[0x7C..0x80], &[0, 0, 0, 0]);
    assert_eq!(&b[0x80..0x84], &[0x24, 0, 0, 0]);
    assert_eq!(&b[0x84..0x88], &[0, 0, 0, 0]);
    assert_eq!(&b[0x10..0x14], &[1, 0, 0, 0]);
}

/// A hireling block without a readable item list saves the empty list
/// (`jf` cannot be omitted while `kf` follows); with the bare marker the
/// loader would take the next marker for the list (22). A list without a block is
/// dropped to the marker alone.
#[test]
fn jf_and_the_hireling_block_agree() {
    let t = Tables;
    let o = ReadOptions {
        expansion: true,
        game: None,
    };
    let mut save = base();
    save.body.as_mut().unwrap().hireling_items = Some(None);
    apply_gaps(&mut save, &Gaps::default());
    assert_eq!(
        save.body.as_ref().unwrap().hireling_items,
        Some(Some(vec![]))
    );
    let f = d2s::write(&save, &t).unwrap();
    assert!(d2s::read(&f, &o, &t).is_ok());

    let mut save = base();
    save.header.hireling = Hireling::default();
    save.body.as_mut().unwrap().hireling_items = Some(Some(vec![item(1)]));
    apply_gaps(&mut save, &Gaps::default());
    assert_eq!(save.body.as_ref().unwrap().hireling_items, Some(None));
    let f = d2s::write(&save, &t).unwrap();
    assert!(d2s::read(&f, &o, &t).is_ok());
}

/// A hot key's item GUID is saved as its
/// 1-based inventory position and loaded back as the GUID at that
/// position; no item, an unknown GUID or a position past the end is
/// "no item".
#[test]
fn hotkey_items_are_saved_as_positions_and_loaded_as_guids() {
    use d2_client::app::save_gaps::{hotkey_slots, loaded_hotkeys};
    use d2_formats::d2s::Slot;
    use d2_server::adapters::handlers::player::HotKey;
    let guids = [40u32, 41, 42];
    let mut keys = [HotKey::UNBOUND; 16];
    keys[0] = HotKey {
        skill: 36,
        left: false,
        item: 42,
    };
    keys[1] = HotKey {
        skill: 7,
        left: true,
        item: u32::MAX,
    };
    keys[2] = HotKey {
        skill: 9,
        left: false,
        item: 99,
    };
    let slots = hotkey_slots(&keys, &guids);
    assert_eq!(slots[0], Slot { code: 36, item: 3 });
    assert_eq!(
        slots[1],
        Slot {
            code: 0x8007,
            item: 0
        }
    );
    assert_eq!(slots[2], Slot { code: 9, item: 0 });
    assert_eq!(slots[3], Slot::NONE);
    let back = loaded_hotkeys(&slots, &guids);
    assert_eq!((back[0].skill, back[0].flag, back[0].item), (36, false, 42));
    assert_eq!(
        (back[1].skill, back[1].flag, back[1].item),
        (7, true, u32::MAX)
    );
    assert_eq!(back[3].skill, -1);
    // An index past the loaded list resolves to −1.
    let past = loaded_hotkeys(&[Slot { code: 5, item: 4 }; 16], &guids);
    assert_eq!(past[0].item, u32::MAX);
}
