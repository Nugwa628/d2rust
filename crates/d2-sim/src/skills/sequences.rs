//! Skill sequences: the player table (compiled from
//! `tables/skills/sequences.tsv`, 23 sequences × 14 COF weapon classes)
//! and its lookup for a player; the monsters'
//! `monseq` sequences ([`MonsterSequences`], and 5).

use std::sync::OnceLock;

use d2_data::bin::BinTable;
use d2_data::tables::Monseq;

/// The table as dumped from the base-game image.
pub const SEQUENCES_TSV: &str = include_str!("../../../../tables/skills/sequences.tsv");

/// COF weapon class codes, `weaponclass.txt` order (`classes`).
pub const CLASSES: [&str; 14] = [
    "hth", "bow", "1hs", "1ht", "stf", "2hs", "2ht", "xbw", "1js", "1jt", "1ss", "1st", "ht1",
    "ht2",
];

/// Player mode codes in `plrmode` row order (a frame's byte +2).
pub const PLAYER_MODES: [&str; 20] = [
    "DT", "NU", "WL", "RN", "GH", "TN", "TW", "A1", "A2", "BL", "SC", "TH", "KK", "S1", "S2", "S3",
    "S4", "DD", "SQ", "KB",
];

/// One 6-byte frame record: +2 drawn mode, +3 drawn frame, +5 event
/// byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeqFrame {
    pub mode: u8,
    pub frame: u8,
    pub event: u8,
}

/// Slot `s` (0–23) per weapon class: the frame list, `None` for a null
/// list (`count` 0).
pub type SeqTable = Vec<[Option<Vec<SeqFrame>>; 14]>;

/// The compiled table (24 slots, slot 0 null).
pub fn table() -> &'static SeqTable {
    static T: OnceLock<SeqTable> = OnceLock::new();
    T.get_or_init(|| parse(SEQUENCES_TSV).expect("tables/skills/sequences.tsv"))
}

/// Parses `sequences.tsv` (columns).
pub fn parse(tsv: &str) -> Result<SeqTable, String> {
    let mut t: SeqTable = (0..24).map(|_| Default::default()).collect();
    for (n, line) in tsv.lines().enumerate().skip(1) {
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 6 {
            return Err(format!("line {}: {} columns", n + 1, cols.len()));
        }
        let s: usize = cols[0]
            .parse()
            .map_err(|_| format!("line {}: seqnum", n + 1))?;
        if s == 0 || s >= t.len() {
            return Err(format!("line {}: seqnum {s}", n + 1));
        }
        let count: usize = cols[2]
            .parse()
            .map_err(|_| format!("line {}: count", n + 1))?;
        let mut frames = Vec::with_capacity(count);
        for f in cols[3].split_whitespace() {
            let (m, i) = f
                .split_once('.')
                .ok_or_else(|| format!("line {}: frame {f}", n + 1))?;
            let mode = PLAYER_MODES
                .iter()
                .position(|c| *c == m)
                .ok_or_else(|| format!("line {}: mode {m}", n + 1))?;
            let frame: u8 = i
                .parse()
                .map_err(|_| format!("line {}: frame {f}", n + 1))?;
            frames.push(SeqFrame {
                mode: mode as u8,
                frame,
                event: 0,
            });
        }
        if frames.len() != count {
            return Err(format!(
                "line {}: {} frames, count {count}",
                n + 1,
                frames.len()
            ));
        }
        for e in cols[4].split(',').filter(|e| !e.is_empty()) {
            let (i, c) = e
                .split_once(':')
                .ok_or_else(|| format!("line {}: event {e}", n + 1))?;
            let i: usize = i
                .parse()
                .map_err(|_| format!("line {}: event {e}", n + 1))?;
            let c: u8 = c
                .parse()
                .map_err(|_| format!("line {}: event {e}", n + 1))?;
            frames
                .get_mut(i)
                .ok_or_else(|| format!("line {}: event index {i}", n + 1))?
                .event = c;
        }
        for c in cols[1].split(',') {
            let k = CLASSES
                .iter()
                .position(|x| *x == c)
                .ok_or_else(|| format!("line {}: class {c}", n + 1))?;
            if t[s][k].is_some() {
                return Err(format!("line {}: class {c} twice", n + 1));
            }
            t[s][k] = (count > 0).then(|| frames.clone());
        }
    }
    Ok(t)
}

/// For a player: the frame list of sequence
/// `seqnum` for COF weapon class `class`; `None` for `seqnum` 0, a slot
/// past the table, a class past the 14 (the base game's fatal assertion
/// at line 0x533), or a null list.
pub fn lookup(seqnum: u8, class: usize) -> Option<&'static [SeqFrame]> {
    table().get(usize::from(seqnum))?.get(class)?.as_deref()
}

/// `DiabPrison`: no sequence for a monster.
const DIAB_PRISON: i32 = 199;

/// The monsters' sequences: per monstats row the slot
/// sequences of its `Skill1`…`Skill8` (the
/// skill u16 at 368 + 2i, the sequence u16 at 392 + 2i written by the
/// `Sk<i>mode` callback, 0xFFFF none) and the `monseq` frame lists by
/// sequence index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonsterSequences {
    /// Per monstats row: (skill, sequence) of slots 0–7, both i16.
    pub slots: Vec<[(i16, i16); 8]>,
    /// The frames of each `monseq` sequence, in row order.
    pub lists: Vec<Vec<SeqFrame>>,
}

impl MonsterSequences {
    /// From the fixed-up `monstats` table and the `monseq` rows.
    pub fn from_tables(monstats: &BinTable, monseq: &[Monseq]) -> Self {
        let i16_at = |r: &[u8], o: usize| {
            r.get(o..o + 2)
                .map_or(-1, |b| i16::from_le_bytes([b[0], b[1]]))
        };
        let slots = monstats
            .iter()
            .map(|r| std::array::from_fn(|i| (i16_at(r, 368 + 2 * i), i16_at(r, 392 + 2 * i))))
            .collect();
        let mut lists: Vec<Vec<SeqFrame>> = Vec::new();
        for m in monseq {
            let k = usize::from(m.sequence);
            if lists.len() <= k {
                lists.resize_with(k + 1, Vec::new);
            }
            lists[k].push(SeqFrame {
                mode: m.mode,
                frame: m.frame,
                event: m.event,
            });
        }
        Self { slots, lists }
    }

    /// For a monster of `class` using `skill`: the slot whose
    /// `Skill<i+1>` is the skill gives the
    /// sequence s; s ≤ 0 or no slot → none; else `monseq` record s.
    // The frame list of sequence s is its `monseq` rows
    // (built from the sorted table, 6-byte
    // rows with the frame layout: +2 mode, +3 frame, +5 event); length
    // and count = their number.
    pub fn lookup(&self, class: usize, skill: i32) -> Option<&[SeqFrame]> {
        if skill == DIAB_PRISON {
            return None;
        }
        let s = self
            .slots
            .get(class)?
            .iter()
            .find(|&&(k, _)| i32::from(k) == skill)
            .map_or(0, |&(_, s)| s);
        if s <= 0 {
            return None;
        }
        let list = self.lists.get(usize::try_from(s).ok()?)?;
        (!list.is_empty()).then_some(list.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_compiles_and_names_every_class_of_each_sequence_once() {
        let t = parse(SEQUENCES_TSV).unwrap();
        // Leap (13): 15 frames S1 0–14, events 5:1, 11:1, 14:1 (table).
        let leap = t[13][0].as_ref().unwrap();
        assert_eq!(leap.len(), 15);
        assert!(leap.iter().all(|f| f.mode == 13));
        let ev: Vec<(usize, u8)> = leap
            .iter()
            .enumerate()
            .filter(|(_, f)| f.event != 0)
            .map(|(i, f)| (i, f.event))
            .collect();
        assert_eq!(ev, vec![(5, 1), (11, 1), (14, 1)]);
        // Whirlwind (10) has no record for bow / xbw.
        assert!(t[10][1].is_none() && t[10][7].is_none());
        assert_eq!(t[10][0].as_ref().map(Vec::len), Some(8));
        assert!(t[0].iter().all(Option::is_none));
    }

    #[test]
    fn the_lookup_refuses_slot_zero_and_classes_past_the_table() {
        assert!(lookup(0, 0).is_none());
        assert!(lookup(13, 14).is_none());
        assert!(lookup(200, 0).is_none());
        assert_eq!(lookup(13, 4).map(<[SeqFrame]>::len), Some(15));
    }

    // The parser reports a wrong frame count.
    #[test]
    fn a_wrong_count_is_an_error() {
        let bad = "seqnum\tclasses\tcount\tframes\tevents\tlist_va\n13\thth\t2\tS1.0\t\t0x1";
        assert!(parse(bad).unwrap_err().contains("count"));
    }

    #[test]
    fn a_monster_takes_the_monseq_list_of_its_skill_slot() {
        let f = |frame, event| SeqFrame {
            mode: 9,
            frame,
            event,
        };
        let m = MonsterSequences {
            // Class 0: slot 1 is skill 306 with sequence 2; slot 0 skill
            // 5 without one; slot 2 skill 199 (DiabPrison) with 2.
            slots: vec![[
                (5, -1),
                (306, 2),
                (199, 2),
                (7, 0),
                (-1, -1),
                (-1, -1),
                (-1, -1),
                (-1, -1),
            ]],
            lists: vec![vec![f(0, 0)], Vec::new(), vec![f(0, 0), f(1, 1)]],
        };
        assert_eq!(m.lookup(0, 306), Some(&[f(0, 0), f(1, 1)][..]));
        // No sequence on the slot (−1), sequence 0, DiabPrison, a skill
        // in no slot, a class past the table: none.
        assert_eq!(m.lookup(0, 5), None);
        assert_eq!(m.lookup(0, 7), None);
        assert_eq!(m.lookup(0, 199), None);
        assert_eq!(m.lookup(0, 8), None);
        assert_eq!(m.lookup(1, 306), None);
        // An empty list: none.
        let mut e = m.clone();
        e.slots[0][1].1 = 1;
        assert_eq!(e.lookup(0, 306), None);
    }

    #[test]
    fn monster_sequences_read_the_slots_and_group_the_rows() {
        use d2_data::bin::BinTable;
        // Two monstats rows: Skill1 at 368, its sequence at 392.
        let mut row = vec![0u8; 408];
        row[368..370].copy_from_slice(&306i16.to_le_bytes());
        row[392..394].copy_from_slice(&1i16.to_le_bytes());
        for i in 1..8 {
            row[368 + 2 * i..370 + 2 * i].copy_from_slice(&(-1i16).to_le_bytes());
            row[392 + 2 * i..394 + 2 * i].copy_from_slice(&(-1i16).to_le_bytes());
        }
        let t = BinTable {
            name: "monstats".into(),
            source: String::new(),
            count: 2,
            record_size: 408,
            records: [row.clone(), row].concat(),
        };
        let rows =
            [(0, 1, 0, 0), (1, 9, 0, 0), (1, 9, 1, 2)].map(|(sequence, mode, frame, event)| {
                Monseq {
                    sequence,
                    mode,
                    frame,
                    dir: 0,
                    event,
                }
            });
        let m = MonsterSequences::from_tables(&t, &rows);
        assert_eq!(m.slots.len(), 2);
        assert_eq!(m.slots[1][0], (306, 1));
        assert_eq!(m.slots[1][7], (-1, -1));
        assert_eq!(m.lists.len(), 2);
        let ev: Vec<u8> = m.lookup(1, 306).unwrap().iter().map(|f| f.event).collect();
        assert_eq!(ev, vec![0, 2]);
    }
}
