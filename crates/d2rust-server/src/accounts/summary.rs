//! A character's summary (what character select and the lobby show) read
//! from its stored record, and the summary of a character that has never
//! been played.
//!
//! The record is the character's save file. The summary reads its header
//! only: the name, the class, the level, the status word and the 32
//! appearance bytes. The unlocked difficulties come from the status word
//! by the shared difficulty rule.

use d2_formats::d2s::{self, status, HEADER_SIZE};
use d2_net::msg::account::{CharSummary, CLASSES};

use super::NewChar;

/// The level byte of a save header.
const LEVEL_AT: usize = 0x2B;
/// The appearance bytes of a save header: 16 component bytes, then 16
/// colour bytes.
const LOOK: std::ops::Range<usize> = 0x88..0xA8;

/// The appearance of a character with nothing equipped: every byte 0xFF,
/// as the save writer fills it before equipped items change it.
pub const BARE_LOOK: [u8; 32] = [0xFF; 32];

/// The summary of a stored record, or why it can't be read.
pub fn of_record(record: &[u8], last_played: u64) -> Result<CharSummary, &'static str> {
    if record.len() < HEADER_SIZE {
        return Err("the record is shorter than a save header");
    }
    let name = std::str::from_utf8(d2s::header_name(record))
        .map_err(|_| "the record's name is not text")?;
    if name.is_empty() {
        return Err("the record has no name");
    }
    let class = d2s::header_class(record);
    if class >= CLASSES {
        return Err("the record's class doesn't exist");
    }
    let status = d2s::header_status(record).ok_or("the record has no status")?;
    Ok(CharSummary {
        name: name.to_owned(),
        class,
        level: record[LEVEL_AT],
        status,
        difficulties: status::difficulties(status),
        look: record[LOOK].to_vec(),
        last_played,
    })
}

/// Where a stored record keeps its status word: the legacy offset for an
/// old save, the current one otherwise (the same rule as
/// [`d2s::header_status`]).
fn status_at(record: &[u8]) -> Option<usize> {
    let version = u32::from_le_bytes(record.get(4..8)?.try_into().ok()?);
    Some(if (1..d2s::VERSION_MIN).contains(&version) {
        0x18
    } else {
        0x24
    })
}

/// Writes `status` into a stored record's header and, for a current-format
/// record, stamps its size and checksum again (a record with a stale
/// checksum doesn't load). `false` when the record has no status word
/// (shorter than its header).
pub fn set_status(record: &mut [u8], status: u16) -> bool {
    let Some(at) = status_at(record) else {
        return false;
    };
    let Some(slot) = record.get_mut(at..at + 2) else {
        return false;
    };
    slot.copy_from_slice(&status.to_le_bytes());
    if at == 0x24 && record.len() >= HEADER_SIZE {
        d2s::finish(record);
    }
    true
}

/// The status word of a new character: its hardcore and expansion flags.
pub fn new_status(hardcore: bool, expansion: bool) -> u16 {
    let mut s = 0;
    if hardcore {
        s |= status::HARDCORE;
    }
    if expansion {
        s |= status::EXPANSION;
    }
    s
}

/// The summary of a character created but never played: level 1, nothing
/// equipped, and the difficulties its progression (normally zero) opens.
pub fn of_new(new: &NewChar) -> CharSummary {
    let status = new_status(new.hardcore, new.expansion) | (u16::from(new.progression & 0x1F) << 8);
    CharSummary {
        name: new.name.clone(),
        class: new.class,
        level: 1,
        status,
        difficulties: status::difficulties(status),
        look: BARE_LOOK.to_vec(),
        last_played: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::d2s::Header;

    #[test]
    fn reads_the_header_fields() {
        let mut h = Header {
            class: 3,
            level: 42,
            status: status::EXPANSION | status::HARDCORE | (10 << 8),
            ..Header::default()
        };
        h.set_name(b"Kashya").unwrap();
        h.components = [1; 16];
        h.colours = [2; 16];
        let mut record = h.to_bytes().to_vec();
        record.extend_from_slice(b"gf body bytes");
        let s = of_record(&record, 77).unwrap();
        assert_eq!(s.name, "Kashya");
        assert_eq!(s.class, 3);
        assert_eq!(s.level, 42);
        assert_eq!(s.status, h.status);
        assert_eq!(s.difficulties, 0b111);
        assert_eq!(s.look, [[1u8; 16], [2u8; 16]].concat());
        assert_eq!(s.last_played, 77);
    }

    #[test]
    fn refuses_what_is_not_a_record() {
        assert!(of_record(&[0; 10], 0).is_err());
        let mut h = Header {
            class: 9,
            ..Header::default()
        };
        h.set_name(b"Kashya").unwrap();
        assert!(of_record(&h.to_bytes(), 0).is_err());
    }

    #[test]
    fn writes_the_status_word_where_the_record_keeps_it() {
        let mut h = Header {
            class: 3,
            status: status::EXPANSION,
            ..Header::default()
        };
        h.set_name(b"Kashya").unwrap();
        let mut record = h.to_bytes().to_vec();
        let wanted = status::EXPANSION | (5 << 8);
        assert!(set_status(&mut record, wanted));
        assert_eq!(d2s::header_status(&record), Some(wanted));
        let mut zeroed = record.clone();
        zeroed[0x0C..0x10].fill(0);
        let stored = u32::from_le_bytes(record[0x0C..0x10].try_into().unwrap());
        assert_eq!(
            d2s::checksum(&zeroed),
            stored,
            "the checksum follows the status"
        );
        assert!(!set_status(&mut record[..3], wanted));
    }

    #[test]
    fn a_new_character_is_level_one_on_normal() {
        let s = of_new(&NewChar {
            name: "Ama".into(),
            class: 0,
            hardcore: true,
            expansion: true,
            progression: 0,
        });
        assert_eq!(s.level, 1);
        assert_eq!(s.status, status::HARDCORE | status::EXPANSION);
        assert_eq!(s.difficulties, 0b001);
        assert_eq!(s.look, BARE_LOOK);
    }

    #[test]
    fn a_new_character_carries_the_progression_it_starts_with() {
        let s = of_new(&NewChar {
            name: "Ama".into(),
            class: 0,
            hardcore: false,
            expansion: true,
            progression: 5,
        });
        assert_eq!(status::progression(s.status), 5);
        assert_eq!(s.difficulties, 0b011, "Nightmare is open at 5");
    }
}
