//! Record lookup, Resurrect insert / remove, talk end, Cain reset and
//! the 9-byte talk messages.
use super::*;

fn opt(string: u16, kind: OptionKind) -> Option<MenuOption> {
    Some(MenuOption { string, kind })
}

fn rec(count: u32, options: [Option<MenuOption>; 5]) -> NpcMenuRecord {
    NpcMenuRecord {
        record: 0,
        npc: 1,
        count,
        options,
        flag: 1,
    }
}

#[test]
fn record_lookup_first_match_else_record_0() {
    let m = NpcMenus::load().unwrap();
    assert_eq!(record_index(m.records(), None), None);
    let i = record_index(m.records(), Some(257)).unwrap();
    assert_eq!(m.records()[i].record, 22);
    assert_eq!(record_index(m.records(), Some(99999)), Some(0));
}

#[test]
fn resurrect_insert_remove() {
    let talk = opt(3381, OptionKind::Talk);
    let hire = opt(3397, OptionKind::Hire);
    let res = opt(STR_RESURRECT_PLACEHOLDER, OptionKind::Resurrect);
    // after the last option
    let mut r = rec(3, [talk, hire, None, None, None]);
    r.options[1] = opt(3396, OptionKind::Trade);
    resurrect_edit(&mut r, true, true);
    assert_eq!(r.count, 4);
    assert_eq!(r.options[2], res);
    // already present: nothing
    resurrect_edit(&mut r, true, true);
    assert_eq!(r.count, 4);
    // before hire
    let mut r = rec(3, [talk, hire, None, None, None]);
    resurrect_edit(&mut r, true, true);
    assert_eq!((r.count, r.options[1], r.options[2]), (4, res, hire));
    // remove
    resurrect_edit(&mut r, false, true);
    assert_eq!((r.count, r.options[1], r.options[2]), (3, hire, None));
    // not an expansion game
    let mut r = rec(3, [talk, hire, None, None, None]);
    resurrect_edit(&mut r, true, false);
    assert_eq!(r.count, 3);
}

#[test]
fn talk_end_flag_rules() {
    assert_eq!(talk_end(true, 148, 1), TalkEnd::RebuildMenu);
    assert_eq!(talk_end(true, 148, 0), TalkEnd::EndInteraction);
    assert_eq!(talk_end(false, 148, 1), TalkEnd::EndInteraction);
    assert_eq!(talk_end(true, 146, 1), TalkEnd::EndInteraction);
}

#[test]
fn talk_messages() {
    assert_eq!(
        msg_chat_start(1, 0x11223344),
        [0x2F, 1, 0, 0, 0, 0x44, 0x33, 0x22, 0x11]
    );
    assert_eq!(
        msg_chat_not_found(0x1FF, 5),
        [0x30, 0xFF, 0, 0, 0, 5, 0, 0, 0]
    );
    assert_eq!(msg_chat_end(5), [0x30, 1, 0, 0, 0, 5, 0, 0, 0]);
    assert_eq!(msg_quest(5, 0x0102), [0x31, 5, 0, 0, 0, 2, 1, 0, 0]);
}

#[test]
fn cain_reset() {
    let m = NpcMenus::load().unwrap();
    let mut recs = m.records().to_vec();
    for r in &mut recs {
        r.count = 5;
    }
    cain_count_reset(&mut recs, 244, 4);
    assert!(recs.iter().all(|r| r.count == 5));
    cain_count_reset(&mut recs, 148, 3);
    assert!(recs.iter().all(|r| r.count == 5));
    cain_count_reset(&mut recs, 245, 6);
    for r in &recs {
        let want = if CAIN_RECORDS.contains(&r.record) {
            2
        } else {
            5
        };
        assert_eq!(r.count, want);
    }
}
