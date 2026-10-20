//! Character panel inputs: the state masks that color
//! the defense and resistance values, the resist penalty, the defense
//! color and the language switch of the panel's language-6–9 rules.

/// The ten colored-value states: (data offset, `states.txt`
/// flag, flag bit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueState {
    ArmBlue,
    RfBlue,
    RcBlue,
    RlBlue,
    RpBlue,
    ArmRed,
    RfRed,
    RcRed,
    RlRed,
    RpRed,
}

impl ValueState {
    pub const ALL: [ValueState; 10] = [
        ValueState::ArmBlue,
        ValueState::RfBlue,
        ValueState::RcBlue,
        ValueState::RlBlue,
        ValueState::RpBlue,
        ValueState::ArmRed,
        ValueState::RfRed,
        ValueState::RcRed,
        ValueState::RlRed,
        ValueState::RpRed,
    ];

    /// The `states.txt` flag column name.
    pub fn flag(self) -> &'static str {
        match self {
            ValueState::ArmBlue => "armblue",
            ValueState::RfBlue => "rfblue",
            ValueState::RcBlue => "rcblue",
            ValueState::RlBlue => "rlblue",
            ValueState::RpBlue => "rpblue",
            ValueState::ArmRed => "armred",
            ValueState::RfRed => "rfred",
            ValueState::RcRed => "rcred",
            ValueState::RlRed => "rlred",
            ValueState::RpRed => "rpred",
        }
    }

    /// The flag bit (`states` `bit`).
    pub fn bit(self) -> u32 {
        match self {
            ValueState::ArmBlue => 19,
            ValueState::RfBlue => 20,
            ValueState::RcBlue => 21,
            ValueState::RlBlue => 22,
            ValueState::RpBlue => 23,
            ValueState::ArmRed => 25,
            ValueState::RfRed => 26,
            ValueState::RcRed => 27,
            ValueState::RlRed => 28,
            ValueState::RpRed => 29,
        }
    }

    /// The data-table offset of the per-flag mask (`+0xCC + 4b`).
    pub fn data_offset(self) -> u32 {
        0xCC + 4 * self.bit()
    }

    /// The state's test function address.
    pub fn test_fn(self) -> u32 {
        match self {
            ValueState::ArmBlue => 0x0063_A550,
            ValueState::RfBlue => 0x0063_A570,
            ValueState::RcBlue => 0x0063_A590,
            ValueState::RlBlue => 0x0063_A5B0,
            ValueState::RpBlue => 0x0063_A5D0,
            ValueState::ArmRed => 0x0063_A5F0,
            ValueState::RfRed => 0x0063_A610,
            ValueState::RcRed => 0x0063_A630,
            ValueState::RlRed => 0x0063_A650,
            ValueState::RpRed => 0x0063_A670,
        }
    }
}

/// 1 when the unit's state bit array
/// (none → 0) and the mask share a bit, over (state count + 31) / 32
/// words.
pub fn states_share_bit(unit_states: Option<&[u32]>, mask: &[u32], state_count: u32) -> bool {
    let Some(states) = unit_states else {
        return false;
    };
    let words = state_count.div_ceil(32) as usize;
    (0..words).any(|i| states.get(i).copied().unwrap_or(0) & mask.get(i).copied().unwrap_or(0) != 0)
}

/// The resist value shown: `stat` + `ResistPenalty` of the
/// client's difficulty record (index clamped to [0, count − 1]) in an
/// expansion game; in a classic game `stat − 20` on difficulty 1, `− 50`
/// on 2, `stat` on 0.
pub fn resist_value(stat: i32, expansion: bool, difficulty: usize, penalties: &[i32]) -> i32 {
    if expansion {
        let i = difficulty.min(penalties.len().saturating_sub(1));
        stat + penalties.get(i).copied().unwrap_or(0)
    } else {
        match difficulty {
            1 => stat - 20,
            2 => stat - 50,
            _ => stat,
        }
    }
}

/// The defense value color: 3 when `armblue`; 3 when the player
/// has an equipped shield (item type 51) and state 101 `holyshield`;
/// then 1 when `armred`.
pub fn defense_color(armblue: bool, shield_and_holyshield: bool, armred: bool) -> u8 {
    let mut c = 0;
    if armblue {
        c = 3;
    }
    if shield_and_holyshield {
        c = 3;
    }
    if armred {
        c = 1;
    }
    c
}

/// The language-6–9 rules of the panel apply only to those ids; English
/// (0) never takes them.
pub fn language_rules(language: u8) -> bool {
    (6..=9).contains(&language)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_masks() {
        let rows: Vec<(u32, u32, &str, u32)> = ValueState::ALL
            .iter()
            .map(|s| (s.test_fn(), s.data_offset(), s.flag(), s.bit()))
            .collect();
        assert_eq!(
            rows,
            [
                (0x0063A550, 0x118, "armblue", 19),
                (0x0063A570, 0x11C, "rfblue", 20),
                (0x0063A590, 0x120, "rcblue", 21),
                (0x0063A5B0, 0x124, "rlblue", 22),
                (0x0063A5D0, 0x128, "rpblue", 23),
                (0x0063A5F0, 0x130, "armred", 25),
                (0x0063A610, 0x134, "rfred", 26),
                (0x0063A630, 0x138, "rcred", 27),
                (0x0063A650, 0x13C, "rlred", 28),
                (0x0063A670, 0x140, "rpred", 29),
            ]
        );
        // the shared-bit test: (count + 31) / 32 words; no array → 0
        assert!(!states_share_bit(None, &[u32::MAX], 40));
        assert!(states_share_bit(Some(&[0b100]), &[0b110], 32));
        assert!(!states_share_bit(Some(&[0b001]), &[0b110], 32));
        // 33 states need two words; a bit in the second word counts
        assert!(states_share_bit(Some(&[0, 1]), &[0, 1], 33));
        // with 32 states only the first word is read
        assert!(!states_share_bit(Some(&[0, 1]), &[0, 1], 32));
        // a short array reads the missing words as 0
        assert!(!states_share_bit(Some(&[0]), &[1, 1], 64));
    }

    #[test]
    fn resist_penalty() {
        // expansion file 0 / −40 / −100
        let exp = [0, -40, -100];
        assert_eq!(resist_value(75, true, 0, &exp), 75);
        assert_eq!(resist_value(75, true, 1, &exp), 35);
        assert_eq!(resist_value(75, true, 2, &exp), -25);
        // the index is clamped to [0, count − 1]
        assert_eq!(resist_value(75, true, 9, &exp), -25);
        assert_eq!(resist_value(5, true, 1, &[]), 5);
        // classic: stat − 20 on difficulty 1, − 50 on 2, stat on 0
        assert_eq!(resist_value(75, false, 0, &[]), 75);
        assert_eq!(resist_value(75, false, 1, &[]), 55);
        assert_eq!(resist_value(75, false, 2, &[]), 25);
    }

    #[test]
    fn defense_color_and_language() {
        assert_eq!(defense_color(false, false, false), 0);
        assert_eq!(defense_color(true, false, false), 3);
        assert_eq!(defense_color(false, true, false), 3);
        // red last: it wins over both blue causes
        assert_eq!(defense_color(true, true, true), 1);
        assert_eq!(defense_color(false, false, true), 1);
        assert!(!language_rules(0));
        assert!(!language_rules(5));
        assert!(language_rules(6) && language_rules(9));
        assert!(!language_rules(10));
    }
}
