//! The NPC talk tables, embedded from `tables/ui/npc-talk-*.tsv`: the 46
//! intro entries (one per NPC and act), their 418 text records, and the 527
//! topic caption pairs the talk menu shows. Each file is line 1 a format
//! line (`# facts v1; …`), line 2 the column names, then tab-separated rows
//! of numbers (string ids and sound ids, never text).

use std::sync::OnceLock;

use super::intro::{GossipRecord, IntroEntry, IntroTable, INTRO_COUNT};

const INTROS: &str = include_str!("../../../../../tables/ui/npc-talk-intros.tsv");
const GOSSIP: &str = include_str!("../../../../../tables/ui/npc-talk-gossip.tsv");
const PAIRS: &str = include_str!("../../../../../tables/ui/npc-talk-pairs.tsv");

/// The data rows of a facts file: the `#` header and the column row are
/// skipped; every row is split at tabs.
fn rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .skip(1)
        .map(|l| l.split('\t').collect())
}

fn num(s: &str) -> u32 {
    match s.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16),
        None => s.parse(),
    }
    .unwrap_or_else(|_| panic!("facts: bad number {s:?}"))
}

/// The 527 (text id, caption id) pairs of the caption table in table order.
pub fn caption_pairs() -> &'static [(u16, u16)] {
    static PAIRS_TABLE: OnceLock<Vec<(u16, u16)>> = OnceLock::new();
    PAIRS_TABLE.get_or_init(|| {
        rows(PAIRS)
            .map(|r| (num(r[1]) as u16, num(r[2]) as u16))
            .collect()
    })
}

/// The NPC intro table with the static values of the facts file
/// and every entry's text records.
pub fn intro_table() -> IntroTable {
    let mut records: Vec<Vec<GossipRecord>> = vec![Vec::new(); INTRO_COUNT];
    for r in rows(GOSSIP) {
        let mut raw = [0u8; 15];
        raw[0..2].copy_from_slice(&(num(r[3]) as u16).to_le_bytes());
        raw[2] = num(r[4]) as u8;
        raw[3..7].copy_from_slice(&num(r[5]).to_le_bytes());
        raw[7..11].copy_from_slice(&num(r[6]).to_le_bytes());
        raw[0x0B..0x0F].copy_from_slice(&num(r[7]).to_le_bytes());
        records[num(r[0]) as usize].push(GossipRecord { raw });
    }
    let entries = rows(INTROS)
        .map(|r| {
            let i = num(r[0]) as usize;
            let mut e =
                IntroEntry::new(num(r[1]), num(r[2]) as u8, std::mem::take(&mut records[i]));
            e.flag13 = num(r[8]) != 0;
            e.no_intro = num(r[9]) != 0;
            e.greeting_due = num(r[10]) != 0;
            e
        })
        .collect();
    IntroTable::new(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::messages::intro::{FLAG_13, NO_INTRO};
    use crate::ui::messages::npc_text::caption_id;

    #[test]
    fn the_facts_give_the_intro_table_and_the_captions() {
        let t = intro_table();
        assert_eq!(t.entries.len(), INTRO_COUNT);
        let mut no: Vec<u32> = t
            .entries
            .iter()
            .filter(|e| e.no_intro)
            .map(|e| e.class)
            .collect();
        no.sort_unstable();
        assert_eq!(no, NO_INTRO);
        let mut f13: Vec<u32> = t
            .entries
            .iter()
            .filter(|e| e.flag13)
            .map(|e| e.class)
            .collect();
        f13.sort_unstable();
        assert_eq!(f13, FLAG_13);
        assert_eq!(
            t.entries.iter().map(|e| e.records.len()).sum::<usize>(),
            418
        );
        // Entry 15 (class 201): 10 records; record 2 is text 255, flag 1.
        let e = &t.entries[15];
        assert_eq!((e.class, e.records.len()), (201, 10));
        assert_eq!((e.records[2].text(), e.records[2].flag()), (255, 1));
        assert!(e.records_are_725cb0);
        // Class 210 gets the introduction topic.
        assert!(!t.entries[t.index_of(210).unwrap()].no_intro);
        // The captions: pair 100 (text 164) and pair 0 (text 64).
        let p = caption_pairs();
        assert_eq!(p.len(), 527);
        assert_eq!(caption_id(p, 164), 3716);
        assert_eq!(caption_id(p, 64), 3714);
        assert_eq!(caption_id(p, 1), 3724);
        // Vector: Akara (148) returns with greeting mode 2 and 4d 94 00.
        let mut t = t;
        t.on_0x91(148);
        let m = t.menu_open(148).unwrap();
        let o = t.greeting_played(m, 148);
        assert_eq!(o.send_4d.unwrap().0, vec![0x4D, 0x94, 0x00]);
    }
}
