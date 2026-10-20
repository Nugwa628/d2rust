//! The names the key-config screen and the belt slot labels show for a key.
//!
//! A key value is a Windows virtual key code, with 0x100-0x104 standing for the extra mouse
//! buttons and wheel directions, and 0xFFFF meaning unbound. Each key has a long name (used on
//! the key-config screen) and a short name (used where space is tight, such as belt labels). A
//! name is either a string id of `string.tbl` or a one-unit string holding the key's low byte
//! (letters and digits). The actual text always comes from the caller's string table, so no game
//! text is held in this module.

/// `KeyNone` ("None"): no key.
pub const KEY_NONE: u16 = 3762;

/// A key's name: a `string.tbl` id, or the one unit of the low byte of
/// the key value (VK 0x31 gives "1", VK 0x41 gives "A").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyName {
    /// The name is the text of this `string.tbl` id.
    Id(u16),
    /// The name is the single UTF-16 unit (the key's low byte).
    Unit(u16),
}

/// The long name's string id, or `None` for a key with no table entry.
fn long_id(vk: u16) -> Option<u16> {
    Some(match vk {
        0x01..=0x04 => 3762 + vk,
        0x08 => 3790,
        0x09 => 3791,
        0x0C => 3792,
        0x0D => 3793,
        0x10 => 3794,
        0x11 => 3795,
        0x12 => 3796,
        0x13 => 3797,
        0x14 => 3798,
        0x15 => 3771,
        0x17..=0x19 => 3772 + (vk - 0x17),
        0x1B => 3775,
        0x1C..=0x1F => 3776 + (vk - 0x1C),
        0x20 => 3799,
        0x21..=0x24 => 3800 + (vk - 0x21),
        0x25..=0x28 => 3780 + (vk - 0x25),
        0x29 => 3784,
        0x2A => 3804,
        0x2B => 3785,
        0x2C..=0x2F => 3805 + (vk - 0x2C),
        0x5B..=0x5D => 3786 + (vk - 0x5B),
        0x60..=0x6F => 3809 + (vk - 0x60),
        0x70..=0x87 => 3825 + (vk - 0x70),
        0x90 => 3789,
        0x91 => 3849,
        0xBA..=0xC0 => 3850 + (vk - 0xBA),
        0xDB..=0xDE => 3857 + (vk - 0xDB),
        0x100 => 3766,
        0x101..=0x104 => 3767 + (vk - 0x101),
        _ => return None,
    })
}

/// The short name's string id.
fn short_id(vk: u16) -> Option<u16> {
    Some(match vk {
        0x08 => 3884,
        0x10 => 3887,
        0x11 => 3888,
        0x1B => 3870,
        0x21 => 3892,
        0x22 => 3893,
        0x2C => 3895,
        0x2D => 3896,
        0x2E => 3897,
        0x60..=0x69 => 3899 + (vk - 0x60),
        0x6A => 3909,
        0x6B => 3910,
        // Table quirk: Separator (0x6C) is "np.", Subtract (0x6D) is "np-", and Decimal (0x6E)
        // has no short name.
        0x6C => 3912,
        0x6D => 3911,
        0x6F => 3913,
        0x90 => 3883,
        0x91 => 3914,
        0x100 => 3861,
        0x101 | 0x102 => 3862 + (vk - 0x101),
        0x103 | 0x104 => 3864 + (vk - 0x103),
        _ => return None,
    })
}

/// The long name of key `vk` (0xFFFF = unbound).
pub fn long_name(vk: u16) -> KeyName {
    if vk == 0xFFFF {
        return KeyName::Id(KEY_NONE);
    }
    long_id(vk).map_or(KeyName::Unit(vk & 0xFF), KeyName::Id)
}

/// The short name of key `vk`; a key with no short name (and 0xFFFF)
/// takes its long name.
pub fn short_name(vk: u16) -> KeyName {
    if vk == 0xFFFF {
        return long_name(vk);
    }
    short_id(vk).map_or_else(|| long_name(vk), KeyName::Id)
}

impl KeyName {
    /// The name as UTF-16 text; `lookup` gives a string id's text (an
    /// id it lacks gives the empty text).
    pub fn text(self, lookup: &dyn Fn(u16) -> Option<Vec<u16>>) -> Vec<u16> {
        match self {
            KeyName::Id(id) => lookup(id).unwrap_or_default(),
            KeyName::Unit(u) => vec![u],
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the key names: the long-name table, the short-name quirks and text lookup.

    use super::*;

    #[test]
    fn long_names_follow_the_table() {
        assert_eq!(long_name(0xFFFF), KeyName::Id(3762));
        assert_eq!(long_name(0x41), KeyName::Unit(0x41));
        assert_eq!(long_name(0x31), KeyName::Unit(0x31));
        assert_eq!(long_name(0x10), KeyName::Id(3794));
        assert_eq!(long_name(0x1B), KeyName::Id(3775));
        assert_eq!(long_name(0x20), KeyName::Id(3799));
        assert_eq!(long_name(0x0D), KeyName::Id(3793));
        assert_eq!(long_name(0x60), KeyName::Id(3809));
        assert_eq!(long_name(0x69), KeyName::Id(3818));
        assert_eq!(long_name(0x70), KeyName::Id(3825));
        assert_eq!(long_name(0x87), KeyName::Id(3848));
        assert_eq!(long_name(0xC0), KeyName::Id(3856));
        assert_eq!(long_name(0xDE), KeyName::Id(3860));
        assert_eq!(long_name(0x100), KeyName::Id(3766));
        assert_eq!(long_name(0x103), KeyName::Id(3769));
        assert_eq!(long_name(0x104), KeyName::Id(3770));
        // Not in the table: the low byte.
        assert_eq!(long_name(0xE5), KeyName::Unit(0xE5));
    }

    // The short table quirk: 0x6C "np.", 0x6D "np-", 0x6E none.
    #[test]
    fn short_names_keep_the_table_quirk() {
        assert_eq!(short_name(0x6C), KeyName::Id(3912));
        assert_eq!(short_name(0x6D), KeyName::Id(3911));
        assert_eq!(short_name(0x6E), long_name(0x6E));
        assert_eq!(short_name(0x6E), KeyName::Id(3823));
        assert_eq!(short_name(0x11), KeyName::Id(3888));
        assert_eq!(short_name(0x1B), KeyName::Id(3870));
        assert_eq!(short_name(0x101), KeyName::Id(3862));
        assert_eq!(short_name(0x104), KeyName::Id(3865));
        assert_eq!(short_name(0x41), KeyName::Unit(0x41));
        assert_eq!(short_name(0xFFFF), KeyName::Id(3762));
        // No short name: the long one.
        assert_eq!(short_name(0x20), long_name(0x20));
    }

    #[test]
    fn text_resolves_through_the_table() {
        let t = |id: u16| (id == 3762).then(|| vec![78, 111]);
        assert_eq!(long_name(0xFFFF).text(&t), vec![78, 111]);
        assert_eq!(long_name(0x41).text(&t), vec![0x41]);
        assert!(long_name(0x10).text(&t).is_empty());
    }
}
