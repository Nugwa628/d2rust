//! The item bit stream of the deferred item messages, built from the real item:
//! [`InvDesk::stream_item`] reads the fields the stream writer
//! (`items::bitstream`) names from their owners (unit record: mode; item
//! store: item data; inventory state: body location, position, the
//! item's own inventory; item tables: items / itemtypes / itemstatcost;
//! stat lists: base, total and the property lists) and
//! [`InvDesk::item_stream`] writes it. Nothing here decides a rule.

use super::inv_world::STAT_SOCKETS;
use super::{InvDesk, InvRest};
use crate::items::bitstream::{self, Kind, StatEntry, StreamItem};
use crate::items::moves::Guid;
use crate::items::ItemTables;
use crate::items::{props, stat, ty, ListKey};
use crate::stats::StatLists;
use crate::stats::{key_layer, key_stat};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::record::Units;
use crate::units::UnitId;
use crate::wiring::economy::find_list;
use crate::wiring::economy::ItemStore;

/// Set-list flags: set items carry their set property lists under flags 0x2040,
/// other items under the plain item flags (0x40).
const SET_FLAGS: u32 = 0x2040;

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// The writer's view of `item` with the sender's flag argument OR-ed
    /// into the item flags and `page` as the page.
    /// `None`: no item unit, record or item data.
    pub fn stream_item(&self, item: Guid, flags: u32, page: u8) -> Option<StreamItem> {
        let u = self.item_unit(item)?;
        let d = self.state.items.get(&u)?;
        let rec = self.econ.tables.item(self.econ.items.get(u)?.record)?;
        let filled = if rec.hasinv != 0 {
            self.state
                .inventories
                .get(&u)
                .map_or(0, |i| i.items().len() as u32)
        } else {
            0
        };
        let mode = self.econ.units.get(u)?.mode;
        let static_pos = match mode {
            3 | 5 => self.econ.hooks.static_position(u),
            _ => None,
        };
        stream_item_of(
            &*self.econ.units,
            &*self.econ.items,
            &*self.econ.stats,
            self.econ.tables,
            u,
            &ItemPlace {
                x: d.x,
                y: d.y,
                body_loc: d.body_loc,
                filled,
                runeword: self.runeword_name(u),
                static_pos,
            },
            flags,
            page,
        )
    }

    /// The runeword record's name string id: the runeword matching the
    /// item's own inventory (class ids in list order) and its socket count
    /// (stat 194 as u8); no record gives 0xFFFF.
    pub fn runeword_name(&self, u: UnitId) -> u16 {
        let Some(it) = self.econ.items.get(u) else {
            return 0xFFFF;
        };
        let fillers: Vec<usize> = self
            .state
            .inventories
            .get(&u)
            .map(|inv| {
                inv.items()
                    .iter()
                    .filter_map(|&f| self.econ.items.get(f).map(|x| x.record))
                    .collect()
            })
            .unwrap_or_default();
        let sockets = self.econ.stats.unit_total(u, STAT_SOCKETS, 0) as u8;
        let t = self.econ.tables;
        props::runeword_row(t, it.record, it.quality, sockets, &fillers)
            .and_then(|row| t.runes.get(row))
            .map_or(0xFFFF, |w| w.name_id)
    }

    /// The stream bytes of `item`, as sent over the network. Empty
    /// when the item has no stream view or the buffer overflows (the
    /// message then carries no stream). The writer's changes to the
    /// item (item level < 1 becomes 1; quality outside 1–9 becomes 2)
    /// are queued in the state and written to the item store by the next
    /// [`InvDesk::apply_write_backs`] (the seam: `MovePending::item_bits`
    /// reads the world only). The overflow check comes after the changes, so
    /// they are kept on overflow too.
    pub fn item_stream(&self, item: Guid, flags: u32, page: u8) -> Vec<u8> {
        self.item_stream_alt(item, flags, page, false)
    }

    /// The store stream of `item`: alt-code exactly when the item
    /// has the vendor flag (unit `flags2`), quality 4–9 and no identified
    /// item flag.
    pub fn store_stream(&self, item: Guid, page: u8) -> Vec<u8> {
        let alt = self.item_unit(item).is_some_and(|u| {
            let vendor = self
                .econ
                .units
                .get(u)
                .is_some_and(|r| r.flags2 & crate::items::moves::deferred::VENDOR_ITEM != 0);
            let it = self.econ.items.get(u);
            let q = it.map_or(0, |i| i.quality);
            let ident = it.is_some_and(|i| i.flags & crate::items::moves::iflag::IDENTIFIED != 0);
            vendor && (4..=9).contains(&q) && !ident
        });
        self.item_stream_alt(item, 0, page, alt)
    }

    fn item_stream_alt(&self, item: Guid, flags: u32, page: u8, alt: bool) -> Vec<u8> {
        let Some(mut view) = self.stream_item(item, flags, page) else {
            return Vec::new();
        };
        view.alt = alt;
        let mut w = bitstream::BitWriter::new(bitstream::BUFFER);
        let wb = bitstream::write_into(&mut w, &view, &self.econ.tables.isc);
        if let Some(u) = self.item_unit(item) {
            self.queue_write_back(u, wb);
        }
        w.finish().unwrap_or_default()
    }

    /// Queues the writer's change of `u` when it
    /// changes anything; [`Self::apply_write_backs`] writes it.
    pub fn queue_write_back(&self, u: UnitId, wb: bitstream::WriteBack) {
        let changed = self
            .econ
            .items
            .get(u)
            .is_some_and(|it| it.ilvl != wb.ilvl || it.quality != wb.quality);
        if changed {
            self.state.write_backs.borrow_mut().push((u, wb));
        }
    }

    /// Writes the queued writer changes (item level, quality) to the item
    /// store, in queue order.
    pub fn apply_write_backs(&mut self) {
        let queued = std::mem::take(self.state.write_backs.get_mut());
        for (u, wb) in queued {
            if let Some(it) = self.econ.items.get_mut(u) {
                it.ilvl = wb.ilvl;
                it.quality = wb.quality;
            }
        }
    }
}

/// Where an item is and what its own data adds to the stream view: the
/// inventory state's fields for [`InvDesk::stream_item`], a monster's
/// holdings for [`crate::wiring::action`]'s add messages.
pub struct ItemPlace {
    /// Grid column of the item in its container.
    pub x: i32,
    /// Grid row of the item in its container.
    pub y: i32,
    /// Body location of an equipped item.
    pub body_loc: u8,
    /// The count of items in the item's own inventory.
    pub filled: u32,
    /// The runeword name string id (see [`InvDesk::runeword_name`]).
    pub runeword: u16,
    /// The static path's position in ground modes (3, 5).
    pub static_pos: Option<(i32, i32)>,
}

/// The writer's view of item unit `u` (see [`InvDesk::stream_item`]).
#[allow(clippy::too_many_arguments)]
pub fn stream_item_of(
    units: &Units,
    items: &ItemStore,
    stats: &StatLists,
    t: &ItemTables,
    u: UnitId,
    d: &ItemPlace,
    flags: u32,
    page: u8,
) -> Option<StreamItem> {
    let r = units.get(u)?;
    let it = items.get(u)?;
    let rec = t.item(it.record)?;
    let is = |k: u16| t.is_type(it.record, k as i16);
    let base = |s: u16| stats.unit_base(u, s, 0);
    let total = |s: u16| stats.unit_total(u, s, 0);
    let list = |state: u16, flags: u32| -> Option<Vec<StatEntry>> {
        let l = find_list(stats, u, ListKey { state, flags })?;
        // A socket filler's property list is linked under the socketed item
        // but is not its own: the filler's root list (flag 0x80000000) is
        // attached to it, which the writer's walk (flag 0x40) never matches.
        if stats.owner_guid(l) != r.guid {
            return None;
        }
        Some(
            stats
                .base_entries(l)
                .into_iter()
                .map(|(k, v)| StatEntry {
                    stat: key_stat(k),
                    param: key_layer(k),
                    value: v,
                })
                .collect(),
        )
    };
    let filled = d.filled;
    let mut sets: [Option<Vec<StatEntry>>; 5] = Default::default();
    for (slot, &s) in sets.iter_mut().zip(bitstream::SET_STATES.iter()) {
        *slot = list(s as u16, SET_FLAGS).or_else(|| list(s as u16, ListKey::ITEM.flags));
    }
    // x / y are the static path's position, the ground sub-tile in modes 3
    // and 5 (the path provider's record), and the grid cell otherwise (the
    // item data).
    let (x, y) = match r.mode {
        3 | 5 => d.static_pos.unwrap_or((d.x, d.y)),
        _ => (d.x, d.y),
    };
    Some(StreamItem {
        flags: it.flags | flags,
        alt: false,
        compact: rec.compactsave != 0,
        // The item format is the stream's version field (101 in
        // expansion games).
        version: it.format,
        mode: r.mode,
        x,
        y,
        body_loc: d.body_loc,
        page,
        code: rec.code,
        base_code: rec.normcode,
        ear_class: it.file_index,
        ear_level: it.ear_level,
        name: it.name,
        filled,
        ilvl: it.ilvl,
        quality: it.quality,
        file_index: it.file_index,
        varinvgfx: t.itype_of(it.record).is_some_and(|y| y.varinvgfx != 0),
        gfx: it.gfx,
        auto_affix: it.auto_affix,
        prefix: it.prefix,
        suffix: it.suffix,
        rare_prefix: it.rare_prefix,
        rare_suffix: it.rare_suffix,
        runeword: d.runeword,
        kind: Kind {
            armor: is(ty::ARMO),
            weapon: is(ty::WEAP),
            gold: is(ty::GOLD),
            charm: is(ty::CHAR),
            body_part: is(ty::BODY) && !is(ty::PLAY),
            scroll_or_book: is(ty::SCRO) || is(ty::BOOK),
        },
        stackable: rec.stackable != 0,
        quest_diff: rec.quest != 0 && rec.questdiffcheck != 0,
        base_defense: base(stat::ARMORCLASS),
        base_max_dur: base(stat::MAXDURABILITY),
        base_sockets: base(stat::NUMSOCKETS),
        total_dur: total(stat::DURABILITY),
        total_gold: total(stat::GOLD),
        total_quantity: total(stat::QUANTITY),
        total_quest_diff: total(stat::QUESTITEMDIFFICULTY),
        main: list(ListKey::ITEM.state, ListKey::ITEM.flags),
        sets,
        runeword_list: list(bitstream::RUNEWORD_STATE as u16, ListKey::ITEM.flags),
        // The trailer bit is 1 exactly when the second realm word is non-zero.
        save_trailer: (it.realm_data[1] != 0).then_some((it.realm_data[0], it.realm_data[1])),
        ..Default::default()
    })
}
