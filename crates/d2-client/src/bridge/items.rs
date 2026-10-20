//! The client's item view for the play preview (gap G16): where each
//! item unit of the model is, read from its last 0x9C / 0x9D record, and
//! the item-move intents the panels and the world view send.
//!
//! The model keeps no inventory nodes: an
//! item unit holds the last message about it. Its stream head
//! (flags, version, mode, location;: the
//! 32-bit code after the location, not for an ear) says where it is:
//! mode 0 stored (grid page), 1 body, 2 belt, 3 / 5 ground, 4 cursor.
//! 0x9C items belong to the local player (the 0x9C handlers use
//! the local player's inventory); 0x9D names its owner.
//!
//! Nothing here decides an outcome: the intents are requests the server
//! checks.

use std::collections::BTreeMap;

use d2_proto::client::{
    DropItem, EquipItem, InsertItemInBuffer, ItemToBelt, PickItem, RemoveItemFromBuffer,
    UseBeltItem,
};
use d2_proto::item_bits::{hflag, BitReader};

use super::world::{ClientWorld, KindData, UnitKey};

/// Item unit type.
pub const ITEM: u8 = 4;

/// Item modes.
pub mod mode {
    pub const STORED: u8 = 0;
    pub const BODY: u8 = 1;
    pub const BELT: u8 = 2;
    pub const GROUND: u8 = 3;
    pub const CURSOR: u8 = 4;
    pub const DROPPING: u8 = 5;
    pub const SOCKETED: u8 = 6;
}

/// One item of the model, as its last record places it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemView {
    pub key: UnitKey,
    /// The items-table code; `None` for an ear.
    pub code: Option<[u8; 4]>,
    /// Header flags F.
    pub flags: u32,
    pub mode: u8,
    /// Body location (mode 1), else 0.
    pub body: u8,
    /// Grid page (mode 0), 0xFF when none.
    pub page: u8,
    /// Grid cell / belt slot (modes 0, 1, 2, 4) or ground sub-tile
    /// (modes 3, 5).
    pub x: u16,
    pub y: u16,
    /// The inventory's unit: the 0x9D owner, else the local player.
    pub owner: Option<UnitKey>,
    /// Shown in an NPC's store: the last record is 0x9C action 11,
    /// not the player's.
    pub store: bool,
    /// The store record's epoch (`ItemRecord::seq`); 0 when not a store item.
    pub store_seq: u32,
    /// The amount of a compact gold pile (`gld`:
    /// after the code a flag bit, then 32 or 12 bits); `None` otherwise.
    pub gold: Option<u32>,
}

/// 0x9C action of an item shown in a store.
pub const ACTION_STORE_SHOWN: u8 = 0x0B;
/// 0x9C action of an item taken out of a store.
pub const ACTION_STORE_TAKEN: u8 = 0x0C;

impl ItemView {
    pub fn on_ground(&self) -> bool {
        matches!(self.mode, mode::GROUND | mode::DROPPING)
    }
}

/// A stream head: flags, mode, body, page, x, y, code.
pub type Head = (u32, u8, u8, u8, u16, u16, Option<[u8; 4]>);

/// The stream head of an item record: flags, mode, location and code.
pub fn peek(stream: &[u8]) -> Option<Head> {
    let mut r = BitReader::new(stream);
    let flags = r.read(32).ok()?;
    let _version = r.read(10).ok()?;
    let m = r.read(3).ok()? as u8;
    let (body, page, x, y) = if matches!(m, mode::GROUND | mode::DROPPING) {
        (0, 0xFF, r.read(16).ok()? as u16, r.read(16).ok()? as u16)
    } else {
        let body = r.read(4).ok()? as u8;
        let x = r.read(4).ok()? as u16;
        let y = r.read(4).ok()? as u16;
        let p = r.read(3).ok()? as u8;
        (body, if p == 0 { 0xFF } else { p - 1 }, x, y)
    };
    let code = if flags & hflag::EAR != 0 {
        None
    } else {
        Some(r.read(32).ok()?.to_le_bytes())
    };
    Some((flags, m, body, page, x, y, code))
}

/// The gold amount of a compact `gld` record.
fn gold_of(stream: &[u8]) -> Option<u32> {
    let mut r = BitReader::new(stream);
    let flags = r.read(32).ok()?;
    if flags & hflag::COMPACT == 0 || flags & hflag::EAR != 0 {
        return None;
    }
    let _version = r.read(10).ok()?;
    let skip = if matches!(r.read(3).ok()? as u8, mode::GROUND | mode::DROPPING) {
        32
    } else {
        15
    };
    r.read(skip).ok()?;
    if r.read(32).ok()?.to_le_bytes() != *b"gld " {
        return None;
    }
    if r.read(1).ok()? == 1 {
        r.read(32).ok()
    } else {
        r.read(12).ok()
    }
}

/// The last record took the item out of the world: RemoveFromContainer,
/// Unequip or RemoveFromBelt with header flag 0x20 ("the item is
/// removed, no write").
fn removed(action: u8, flags: u32) -> bool {
    action == ACTION_STORE_TAKEN || matches!(action, 0x05 | 0x08 | 0x0F) && flags & 0x20 != 0
}

/// The view of item unit `key`, `None` when it is not an item with a
/// readable record or the record removed it.
pub fn item(w: &ClientWorld, key: UnitKey) -> Option<ItemView> {
    let u = w.units.get(&key)?;
    let KindData::Item(d) = &u.kind else {
        return None;
    };
    let r = d.last.as_ref()?;
    let (flags, m, body, page, x, y, code) = peek(&r.stream)?;
    if removed(r.action, flags) {
        return None;
    }
    Some(ItemView {
        key,
        code,
        flags,
        mode: m,
        body,
        page,
        x,
        y,
        owner: if r.action == ACTION_STORE_SHOWN {
            w.interact_npc.or(r.owner)
        } else {
            r.owner
        }
        .or(w.local_player),
        store: r.action == ACTION_STORE_SHOWN,
        store_seq: r.seq,
        gold: gold_of(&r.stream),
    })
}

/// The last item bit stream of item unit `key` (the bytes after the
/// 0x9C / 0x9D head), for the readers that decode more than the head.
pub fn stream(w: &ClientWorld, key: UnitKey) -> Option<&[u8]> {
    match &w.units.get(&key)?.kind {
        KindData::Item(d) => d.last.as_ref().map(|r| r.stream.as_slice()),
        _ => None,
    }
}

/// Every item of the model in key order.
pub fn items(w: &ClientWorld) -> Vec<ItemView> {
    w.units
        .keys()
        .filter(|k| k.unit_type == ITEM)
        .filter_map(|&k| item(w, k))
        .collect()
}

/// The local player's items that are not on the ground, by mode.
pub fn local_items(w: &ClientWorld) -> Vec<ItemView> {
    let Some(p) = w.local_player else {
        return Vec::new();
    };
    items(w)
        .into_iter()
        .filter(|i| !i.on_ground() && !i.store && i.owner == Some(p))
        .collect()
}

/// The items shown in the NPC's store, in key order (their GUIDs rise in
/// store order).
pub fn store_items(w: &ClientWorld) -> Vec<ItemView> {
    items(w).into_iter().filter(|i| i.store).collect()
}

/// The ground items.
pub fn ground_items(w: &ClientWorld) -> Vec<ItemView> {
    items(w).into_iter().filter(ItemView::on_ground).collect()
}

/// The local player's cursor item (`cursor_item`).
pub fn cursor_item(w: &ClientWorld) -> Option<ItemView> {
    let p = w.local()?;
    let KindData::Player(d) = &p.kind else {
        return None;
    };
    item(w, UnitKey::new(ITEM, d.cursor_item?))
}

/// The local player's belt: slot (x) → item.
pub fn belt(w: &ClientWorld) -> BTreeMap<u16, ItemView> {
    local_items(w)
        .into_iter()
        .filter(|i| i.mode == mode::BELT)
        .map(|i| (i.x, i))
        .collect()
}

/// The item-table facts the preview draws with: inventory size and the
/// art files (`items/*.txt`: `invwidth`, `invheight`, `invfile`,
/// `flippyfile`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemArtRow {
    pub inv_w: u8,
    pub inv_h: u8,
    pub inv_file: String,
    pub flippy_file: String,
    /// The itemtypes `beltable` of the item's type and a 1 x 1 inventory
    /// size: what
    /// the server's belt test reads.
    pub beltable: bool,
}

/// Item art rows by code (weapons, armor, misc; the first row of a code).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemArtRows(pub BTreeMap<[u8; 4], ItemArtRow>);

impl ItemArtRows {
    pub fn get(&self, code: [u8; 4]) -> Option<&ItemArtRow> {
        self.0.get(&code)
    }
}

/// C→S 0x16 PickItem: pick `item` up; cursor
/// 0 → auto placement, else to the cursor.
pub fn pick(item: u32, to_cursor: bool) -> PickItem {
    PickItem {
        type_: u32::from(ITEM),
        id: item,
        cursor: u32::from(to_cursor),
    }
}

/// C→S 0x17 DropItem: drop the cursor item.
pub fn drop(item: u32) -> DropItem {
    DropItem { item }
}

/// C→S 0x18 InsertItemInBuffer: the cursor item into a grid page.
pub fn insert(item: u32, x: u32, y: u32, page: u32) -> InsertItemInBuffer {
    InsertItemInBuffer { item, x, y, page }
}

/// C→S 0x19 RemoveItemFromBuffer: a grid item to the cursor.
pub fn remove(item: u32) -> RemoveItemFromBuffer {
    RemoveItemFromBuffer { item }
}

/// C→S 0x1A EquipItem: the cursor item to a body location.
pub fn equip(item: u32, body: u8) -> EquipItem {
    EquipItem {
        item,
        bodyloc: body,
    }
}

/// C→S 0x23 ItemToBelt: the cursor item to a belt slot.
pub fn to_belt(item: u32, slot: u32) -> ItemToBelt {
    ItemToBelt { item, slot }
}

/// C→S 0x20 UseGridItem: use a stored item at the world point
/// (x, y) (the player's own subtile position, within range of the
/// server's check); the cube is opened this way.
pub fn use_grid(item: u32, x: u32, y: u32) -> d2_proto::client::UseGridItem {
    d2_proto::client::UseGridItem { item, x, y }
}

/// C→S 0x26 UseBeltItem: use a belt item on the player.
pub fn use_belt(item: u32) -> UseBeltItem {
    UseBeltItem { item, on_merc: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{ClientUnit, ItemData, ItemRecord, PlayerData};

    /// A stream head: flags, version 0x65, mode, location, code.
    pub(crate) fn stream(flags: u32, m: u8, loc: (u8, u16, u16, u8), code: &[u8; 4]) -> Vec<u8> {
        let mut bits: Vec<(u32, u32)> = vec![(flags, 32), (0x65, 10), (u32::from(m), 3)];
        if matches!(m, 3 | 5) {
            bits.extend([(u32::from(loc.1), 16), (u32::from(loc.2), 16)]);
        } else {
            bits.extend([
                (u32::from(loc.0), 4),
                (u32::from(loc.1), 4),
                (u32::from(loc.2), 4),
                (u32::from(loc.3), 3),
            ]);
        }
        bits.push((u32::from_le_bytes(*code), 32));
        let mut out = Vec::new();
        let mut acc: u64 = 0;
        let mut n = 0;
        for (v, w) in bits {
            acc |= u64::from(v) << n;
            n += w;
            while n >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            out.push(acc as u8);
        }
        out
    }

    fn world_with(items: &[(u32, u8, Vec<u8>)]) -> ClientWorld {
        let mut w = ClientWorld::default();
        let p = UnitKey::new(0, 1);
        let mut u = ClientUnit::new(p);
        u.kind = KindData::Player(PlayerData::default());
        w.units.insert(p, u);
        w.local_player = Some(p);
        for (guid, action, s) in items {
            let k = UnitKey::new(ITEM, *guid);
            let mut u = ClientUnit::new(k);
            u.kind = KindData::Item(ItemData {
                last: Some(ItemRecord {
                    id: 0x9C,
                    action: *action,
                    category: 0,
                    owner: None,
                    seq: 0,
                    stream: s.clone(),
                }),
                ..ItemData::default()
            });
            w.units.insert(k, u);
        }
        w
    }

    #[test]
    fn the_head_places_grid_belt_body_and_ground_items() {
        let w = world_with(&[
            (10, 0x04, stream(0x10, 0, (0, 2, 3, 1), b"hp1 ")),
            (11, 0x0E, stream(0x10, 2, (0, 1, 0, 0), b"hp2 ")),
            (12, 0x06, stream(0x10, 1, (4, 0, 0, 0), b"blad")),
            (13, 0x03, stream(0x10, 3, (0, 200, 300, 0), b"cap ")),
        ]);
        let all = items(&w);
        assert_eq!(all.len(), 4);
        assert_eq!((all[0].mode, all[0].page, all[0].x, all[0].y), (0, 0, 2, 3));
        assert_eq!(all[0].code, Some(*b"hp1 "));
        assert_eq!(belt(&w).keys().copied().collect::<Vec<_>>(), [1]);
        assert_eq!(all[2].body, 4);
        let g = ground_items(&w);
        assert_eq!((g.len(), g[0].x, g[0].y), (1, 200, 300));
        assert_eq!(local_items(&w).len(), 3);
    }

    #[test]
    fn a_removal_with_flag_0x20_hides_the_item_and_the_cursor_reads_it() {
        let mut w = world_with(&[
            (20, 0x0F, stream(0x20, 2, (0, 0, 0, 0), b"hp1 ")),
            (21, 0x12, stream(0x10, 4, (0, 0, 0, 0), b"cap ")),
        ]);
        assert!(item(&w, UnitKey::new(ITEM, 20)).is_none());
        if let Some(KindData::Player(d)) = w.units.get_mut(&UnitKey::new(0, 1)).map(|u| &mut u.kind)
        {
            d.cursor_item = Some(21);
        }
        assert_eq!(cursor_item(&w).map(|i| i.code), Some(Some(*b"cap ")));
    }

    #[test]
    fn intents_carry_the_item_guid() {
        assert_eq!(
            crate::bridge::intent::encode(&pick(7, false)),
            [0x16, 4, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(crate::bridge::intent::encode(&drop(7)), [0x17, 7, 0, 0, 0]);
    }
}
