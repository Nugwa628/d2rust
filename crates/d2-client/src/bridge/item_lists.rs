//! The stat lists of the item units: each
//! item owns the property lists of its 0x9C / 0x9D stream, and the
//! lists attached to a unit are added to its total
//! ([`ClientWorld::total`]), the item's base array (weapon
//! speed, damage, defense, block) with them. The server's stat messages
//! carry the base only; the client sums the equipped items itself.
//!
//! The decoded properties are kept in the item's kind data when the
//! record arrives ([`refresh`]); whether they count is decided when the
//! total is read ([`attached_to`]), from the item's last record, so a
//! move out of the body or grid detaches the list and a move
//! onto the body attaches it with no extra bookkeeping.
//!
//! PROVISIONAL (open question 2): which of an
//! item's lists are attached. D2RUST adds every list the stream carries
//! (base-magic list, the set lists of the active set mask, the runeword
//! list), the set-bonus lists included; the base game's choice per set
//! item is not known; open until the client's stat totals are compared
//! against the S→C 0x1D-0x1F messages with two set pieces and a magic
//! ring worn (the runeword list has no scene yet).
//! The attach condition is D2RUST's reading
//! of the attach step: body locations 1-10 and 13+ (not the swap set 11 / 12),
//! item flag 0x4000 clear, plus charms on
//! inventory page 0 (the server links those).
//! Socketed gems and runes apply through and are
//! not added here.

use std::sync::Arc;

use super::items::{self, mode};
use super::world::{ClientWorld, KindData, UnitKey};

/// One property of an item's list: stat, layer (the stream's param) and
/// the list value (the stream value less `Save Add`, shifted by
/// `ValShift`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemProp {
    pub stat: u16,
    pub layer: u16,
    pub value: i32,
}

/// The item data the reader writes from a
/// record: item data offsets +0x00
/// quality, +0x18 flags, +0x28 file index, +0x2C level, +0x32 / +0x34
/// rare names, +0x36 auto affix, +0x38 / +0x3E affix slots, and the base
/// array of the item's list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemFields {
    /// Items-table row of the record's code (unit class).
    pub class: u32,
    pub quality: u32,
    pub flags: u32,
    pub file_index: i32,
    pub ilvl: u32,
    pub auto: u16,
    pub prefix: [u16; 3],
    pub suffix: [u16; 3],
    pub rare: (u16, u16),
    /// Base array entries, (stat, layer) order.
    pub base: Vec<ItemProp>,
}

/// Decodes an item stream into its properties and whether the item is a
/// charm; implemented over the game's item tables outside the bridge
/// (`app::items::TableDecoder`), since the bridge names no server crate.
pub trait StreamProps: Send + Sync {
    /// Empty when the stream does not decode (a compact record has none).
    fn props(&self, stream: &[u8]) -> (Vec<ItemProp>, bool);
    /// "Item with items-table `code` is of type `t`" (with
    /// equivalence); the type test of `use_state` test 5.
    fn item_is_type(&self, _code: [u8; 4], _t: i16) -> bool {
        false
    }
    /// The item data of the record (`None`: does not decode).
    fn fields(&self, _stream: &[u8]) -> Option<ItemFields> {
        None
    }
    /// `rangeadder` of the items-table row `code` when the item is a
    /// weapon (type `weap`), else `None` (reads the row's
    /// byte +0x104; takes a worn item of type 0x2D).
    fn weapon_range_adder(&self, _code: [u8; 4]) -> Option<i32> {
        None
    }
    /// The items-table row `code`'s `missiletype` (+0xFA)
    /// and primary `type` (+0x11E); `None` without a row.
    fn missile_and_type(&self, _code: [u8; 4]) -> Option<(u16, i16)> {
        None
    }
    /// The type class of the items-table row `code`: its
    /// `wclass` as a `sequences::CLASSES` index when that is 1–7 or 12,
    /// else 0 (as the attack weapon).
    fn type_class(&self, _code: [u8; 4]) -> u32 {
        0
    }
    /// items `component` (+0x115) of the row `code` (0 without a row).
    fn component(&self, _code: [u8; 4]) -> u8 {
        0
    }
    /// The itemtypes `throwable` (+0x10) of the row
    /// `code`'s primary type; false
    /// without a row.
    fn type_throwable(&self, _code: [u8; 4]) -> bool {
        false
    }
    /// The itemtypes `shoots` (+0x0C) of the row `code`'s
    /// primary type, an itemtypes row (0 none or without a row); the ammo
    /// rule of the item type test (`hand`).
    fn type_shoots(&self, _code: [u8; 4]) -> i16 {
        0
    }
    /// Whether items `stackable` of the row `code` is above 0 (false
    /// without a row).
    fn stackable(&self, _code: [u8; 4]) -> bool {
        false
    }
}

/// The decoder the model uses (`None` until the play app installs it).
/// Compares equal always: it is a handle, not model state.
#[derive(Clone, Default)]
pub struct ItemTablesRef(pub Option<Arc<dyn StreamProps>>);

impl std::fmt::Debug for ItemTablesRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ItemTablesRef({})", self.0.is_some())
    }
}

impl PartialEq for ItemTablesRef {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for ItemTablesRef {}

/// Decodes the last record of item `key` into its kind data. Called
/// after each 0x9C / 0x9D record is stored.
pub fn refresh(w: &mut ClientWorld, key: UnitKey) {
    // A new record re-adds an item 0x92 unlinked.
    if let Some(KindData::Item(d)) = w.units.get_mut(&key).map(|u| &mut u.kind) {
        d.unlinked = false;
        d.stats_off = false;
        // The record read writes item data +0x18 := the
        // stream's flag word (its first 32 bits) less 0x2000000 and
        // 0x80000 (step 3): the requirement
        // flag 0x4000 the server's inventory pass set arrives with the
        // record, and the equip then leaves the item's list
        // detached.
        if let Some(f) = d.last.as_ref().and_then(|r| r.stream.get(..4)) {
            let f = u32::from_le_bytes([f[0], f[1], f[2], f[3]]) & !0x0208_0000;
            d.flags4 = f & 4 != 0;
            d.flags = f & !4;
        }
    }
    let Some(t) = w.item_tables.0.clone() else {
        return;
    };
    let Some(KindData::Item(d)) = w.units.get_mut(&key).map(|u| &mut u.kind) else {
        return;
    };
    let (props, charm) = match &d.last {
        Some(r) => t.props(&r.stream),
        None => (Vec::new(), false),
    };
    d.props = props;
    d.charm = charm;
    d.fields = match &d.last {
        Some(r) => t.fields(&r.stream),
        None => None,
    };
}

/// The unit whose list the item's properties are attached to, `None`
/// while detached.
pub fn attached_to(w: &ClientWorld, key: UnitKey) -> Option<UnitKey> {
    let KindData::Item(d) = &w.units.get(&key)?.kind else {
        return None;
    };
    if d.props.is_empty() || d.flags & 0x4000 != 0 || d.unlinked {
        return None;
    }
    let v = items::item(w, key)?;
    if v.store || v.on_ground() {
        return None;
    }
    let active = match v.mode {
        // The swap set (11 / 12) has its list detached.
        mode::BODY => !matches!(v.body, 11 | 12),
        mode::STORED => d.charm && v.page == 0,
        _ => false,
    };
    active.then_some(v.owner).flatten()
}

impl ClientWorld {
    /// The sum of `stat` / `layer` over the item lists attached to
    /// `unit`, each item's base array included.
    pub fn item_lists_total(&self, unit: UnitKey, stat: u16, layer: u16) -> i32 {
        self.units
            .iter()
            .filter(|(k, _)| k.unit_type == items::ITEM)
            .filter(|(&k, _)| attached_to(self, k) == Some(unit))
            .filter_map(|(_, u)| match &u.kind {
                KindData::Item(d) if !d.stats_off => Some(d),
                _ => None,
            })
            // The item's base array counts with its lists:
            // weapon speed 68, defense 31, damage 21–24, ...
            .flat_map(|d| {
                d.props
                    .iter()
                    .chain(d.fields.iter().flat_map(|f| f.base.iter()))
            })
            .filter(|p| p.stat == stat && p.layer == layer)
            .fold(0i32, |a, p| a.wrapping_add(p.value))
    }

    /// Installs the decoder the streams are decoded with.
    pub fn set_item_tables(&mut self, t: Arc<dyn StreamProps>) {
        self.item_tables = ItemTablesRef(Some(t));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::msg::support::Model;
    use crate::bridge::world::{PlayerData, PLAYER};

    /// The 0x9D (action 6, equip) of Firelizard's Talons on the level-30
    /// assassin of a recorded game, byte for byte as the base game sent
    /// it: stream flags 0x00804011, so the
    /// requirement flag 0x4000 the server's inventory pass set.
    const TALONS_9D: &str = "9d 06 32 05 01 00 00 00 00 01 00 00 00 11 40 80 00 65 84 08 \
        70 c3 76 07 02 cf 01 17 00 11 c6 8d 3b 21 06 83 1d 7c 97 d1 1a 83 59 03 51 bc 64 00 f6 1f";

    fn worn(flag_byte: &str) -> (Model, UnitKey, UnitKey) {
        let p = UnitKey::new(PLAYER, 1);
        let mut m = Model::default();
        m.put(p).kind = KindData::Player(PlayerData::default());
        m.w.local_player = Some(p);
        m.hex(&TALONS_9D.replacen("11 40 80 00", flag_byte, 1));
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let k = UnitKey::new(items::ITEM, 1);
        let KindData::Item(d) = &mut m.w.units.get_mut(&k).unwrap().kind else {
            panic!("item data");
        };
        // Fire resistance 39 (+30), as the table decoder would give it.
        d.props = vec![ItemProp {
            stat: 39,
            layer: 0,
            value: 30,
        }];
        (m, p, k)
    }

    #[test]
    fn a_record_with_the_requirement_flag_stays_detached() {
        let (m, p, k) = worn("11 40 80 00");
        let KindData::Item(d) = &m.unit(k).kind else {
            panic!("item data");
        };
        // +0x18 := the stream's flags (0x2000000 / 0x80000 off).
        assert_eq!(d.flags, 0x0080_4011);
        assert!(!d.flags4);
        assert_eq!(attached_to(&m.w, k), None);
        assert_eq!(m.w.item_lists_total(p, 39, 0), 0);
        // The same record without 0x4000 attaches to its owner.
        let (m, p, k) = worn("11 00 80 00");
        assert_eq!(attached_to(&m.w, k), Some(p));
        assert_eq!(m.w.item_lists_total(p, 39, 0), 30);
    }
}
