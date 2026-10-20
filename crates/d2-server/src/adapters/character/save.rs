//! Character storage, write side: the parts of a save the writer rebuilds from
//! the game at save time rather than from the loaded file: the appearance bytes
//! and the hireling's item list (`jf`; the load restores it again when the
//! hireling is restored).
//!
//! The rules are `d2_formats::d2s` (header, sections, the fill) and
//! `d2_sim::items::bitstream` (one item entry). What is decided here is
//! which game values feed them: the item list in the writer's order, the
//! items a writer skips, each item's children and the appearance inputs of
//! each inventory item, read through [`SaveItems`] ([`InvDesk`] in the game).
//! The server's save path and `d2s-tool resave` use it.

use d2_formats::d2s::appearance::{
    self, AppearanceTables, Equipment, EquippedItem, BODY_SLOTS, MODE_EQUIPPED,
};
use d2_formats::d2s::{D2s, ItemEntry};
use d2_sim::items::bitstream::{write_save, IscTable, StreamItem, WriteBack};
use d2_sim::items::inventory::node;
use d2_sim::items::ItemTables;
use d2_sim::units::lifecycle::LifecycleHooks;
use d2_sim::units::UnitId;
use d2_sim::wiring::inventory::{InvDesk, InvRest};

/// The flags-ex bit of an item that writes 0 bytes.
pub const FLAGS2_NO_SAVE: u32 = 0x8000;

/// What the writer reads of the units and items (a view of the
/// inventory state, the item store and the unit records).
pub trait SaveItems {
    /// The owner's inventory item list, in link order (an item's own
    /// inventory gives its children).
    fn items(&self, owner: UnitId) -> Vec<UnitId>;
    /// The item at body location `loc` (grid 0).
    fn body_item(&self, owner: UnitId, loc: u8) -> Option<UnitId>;
    /// The weapon in use.
    fn weapon_in_use(&self, owner: UnitId) -> Option<UnitId>;
    /// The cursor item.
    fn cursor(&self, owner: UnitId) -> Option<UnitId>;
    /// The item's node kind and body location.
    fn node(&self, item: UnitId) -> Option<(u8, u8)>;
    /// The unit's flags-ex word.
    fn flags2(&self, item: UnitId) -> u32;
    /// The item's stream view without children; `None`: no item unit.
    fn stream(&self, item: UnitId) -> Option<StreamItem>;
    /// The item's appearance inputs, with
    /// `first_child` unset (the caller fills it from [`SaveItems::items`]).
    fn appearance(&self, item: UnitId) -> Option<EquippedItem>;
    /// Queues the writer's change of `item` (item level < 1 → 1,
    /// quality outside 1–9 → 2); the owner
    /// of the model applies it (`InvDesk::apply_write_backs`).
    fn write_back(&self, _item: UnitId, _wb: WriteBack) {}
}

/// Why a save section could not be written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SaveError {
    /// An item entry that does not fit the save buffer.
    #[error("item entry overflows the save buffer (formats/d2s.md §8.1 rule 5)")]
    Overflow,
    /// The model has no body (the 335-byte stub) or no such item.
    #[error("{0}")]
    Model(&'static str),
}

/// The save of the game's state at save time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveContext {
    /// Expansion game (the save carries the `jf` / `kf` sections).
    pub expansion: bool,
    /// The player's weapon class index (see `hand_weapon_class`).
    pub weapon_class: i32,
    /// The states the player has.
    pub states: Vec<u32>,
}

/// One item entry: the item's stream, then each item of its
/// own inventory, recursively; `units` gets the written items in
/// write order (the order of [`write_save`]'s write-backs).
fn stream_tree(src: &dyn SaveItems, item: UnitId, units: &mut Vec<UnitId>) -> Option<StreamItem> {
    let mut s = src.stream(item)?;
    units.push(item);
    s.children = src
        .items(item)
        .into_iter()
        .filter_map(|c| stream_tree(src, c, units))
        .collect();
    Some(s)
}

/// The items of `owner`'s list in the writer's order:
/// every list item except a body-node item at location 4 or 5, then the
/// hands (no weapon in use or the right hand in use → right, left; else
/// left, right), then the cursor item.
pub fn list_order(src: &dyn SaveItems, owner: UnitId) -> Vec<UnitId> {
    let mut out: Vec<UnitId> = src
        .items(owner)
        .into_iter()
        .filter(|&u| {
            !matches!(src.node(u), Some((node::BODY, loc))
                if loc == appearance::body::RIGHT_HAND || loc == appearance::body::LEFT_HAND)
        })
        .collect();
    let right = src.body_item(owner, appearance::body::RIGHT_HAND);
    let left = src.body_item(owner, appearance::body::LEFT_HAND);
    let hands = match src.weapon_in_use(owner) {
        None => [right, left],
        Some(w) if Some(w) == right => [right, left],
        Some(_) => [left, right],
    };
    out.extend(hands.into_iter().flatten());
    out.extend(src.cursor(owner));
    out
}

/// The item list of `owner` as save entries: items
/// with flags-ex bit 0x8000 write nothing and are not counted.
pub fn item_list(
    src: &dyn SaveItems,
    owner: UnitId,
    isc: &dyn IscTable,
) -> Result<Vec<ItemEntry>, SaveError> {
    let mut out = Vec::new();
    for u in list_order(src, owner) {
        if src.flags2(u) & FLAGS2_NO_SAVE != 0 {
            continue;
        }
        let mut units = Vec::new();
        let s = stream_tree(src, u, &mut units)
            .ok_or(SaveError::Model("item without a stream view"))?;
        let (bytes, wbs) = write_save(&s, isc).map_err(|_| SaveError::Overflow)?;
        // The writer's changes stay on every written item, children too.
        for (u, wb) in units.into_iter().zip(wbs) {
            src.write_back(u, wb);
        }
        out.push(ItemEntry { bytes });
    }
    Ok(out)
}

/// The appearance provider: the
/// player's item list with each item's inputs, the body grid slots 0–10
/// and the weapon in use as indices into that list.
pub fn equipment(src: &dyn SaveItems, player: UnitId, ctx: &SaveContext) -> Equipment {
    let list = src.items(player);
    let items = list
        .iter()
        .map(|&u| {
            let mut e = src.appearance(u).unwrap_or_default();
            e.first_child = src
                .items(u)
                .first()
                .and_then(|&c| src.appearance(c))
                .map(|c| c.record);
            e
        })
        .collect();
    let at = |u: Option<UnitId>| u.and_then(|u| list.iter().position(|&x| x == u));
    let mut body_grid = [None; BODY_SLOTS];
    for (loc, slot) in body_grid.iter_mut().enumerate() {
        *slot = at(src.body_item(player, loc as u8));
    }
    Equipment {
        items,
        body_grid,
        weapon_in_use: at(src.weapon_in_use(player)),
        weapon_class: ctx.weapon_class,
        states: ctx.states.clone(),
    }
}

/// The parts of `save` the writer rebuilds from the game: the appearance
/// bytes of `player` and, for a full save, the `jf`
/// section (in an expansion game the marker, then the item
/// list of `hireling` when the player has a hireling node).
pub fn rebuild(
    save: &mut D2s,
    src: &dyn SaveItems,
    player: UnitId,
    hireling: Option<UnitId>,
    ctx: &SaveContext,
    t: &AppearanceTables,
    isc: &dyn IscTable,
) -> Result<(), SaveError> {
    save.header
        .rebuild_appearance(&equipment(src, player, ctx), t);
    if let Some(body) = save.body.as_mut() {
        let list = hireling.map(|h| item_list(src, h, isc)).transpose()?;
        body.set_hireling_items(ctx.expansion, list);
    }
    Ok(())
}

/// The appearance inputs of a save's own player item list, without a
/// running game (`d2s-tool resave`): each
/// entry read as the load reads it (`items::bitstream::read`), the body
/// grid from the equipped items' body locations, and the weapon in use as
/// the load leaves it: the body link run on each equipped hand item in list
/// order. No states (a loaded player has none). The weapon class is `hand_weapon_class`.
pub fn equipment_of_save(
    items: &[ItemEntry],
    t: &ItemTables,
    a: &AppearanceTables,
) -> Result<Equipment, String> {
    use d2_sim::items::bitstream::read::read_save_entry;
    use d2_sim::items::inventory::{body, iflag, ty};
    let mut out = Vec::new();
    let mut locs = Vec::new();
    for (k, e) in items.iter().enumerate() {
        let r = read_save_entry(&e.bytes, t).map_err(|err| format!("item {k}: {err:?}"))?;
        let s = &r.item.item;
        out.push(EquippedItem {
            record: r.item.record,
            mode: s.mode as u8,
            body_loc: s.body_loc,
            quality: s.quality,
            prefix: s.prefix,
            suffix: s.suffix,
            auto_affix: s.auto_affix,
            file_index: s.file_index,
            flags: s.flags,
            max_sockets: max_sockets(t, r.item.record, s.ilvl),
            first_child: r.children.first().map(|c| c.item.record),
        });
        locs.push((s.mode, s.body_loc, s.flags, r.item.record));
    }
    let mut body_grid = [None; BODY_SLOTS];
    for (k, &(mode, loc, _, _)) in locs.iter().enumerate() {
        if mode == u32::from(MODE_EQUIPPED) && usize::from(loc) < BODY_SLOTS {
            body_grid[usize::from(loc)] = Some(k);
        }
    }
    let usable = |f: u32| f & iflag::IDENTIFIED != 0 && f & (iflag::BROKEN | iflag::F4000) == 0;
    let weap = |rec: usize| t.is_type(rec, ty::WEAP);
    let tpot = |rec: usize| t.item(rec).is_some_and(|r| r.type_ == ty::TPOT);
    let mut in_use: Option<usize> = None;
    for (k, &(mode, loc, flags, rec)) in locs.iter().enumerate() {
        let hand = matches!(loc, body::RIGHT_HAND | body::LEFT_HAND);
        if mode != u32::from(MODE_EQUIPPED) || !hand || !weap(rec) {
            continue;
        }
        if !usable(flags) {
            if in_use == Some(k) {
                in_use = None;
            }
            continue;
        }
        in_use = match in_use {
            None => Some(k),
            Some(c) if !weap(locs[c].3) || tpot(locs[c].3) => Some(k),
            Some(c) if c == k => None,
            keep => keep,
        };
    }
    let weapon_class = hand_weapon_class(&body_grid, &locs, a);
    Ok(Equipment {
        items: out,
        body_grid,
        weapon_in_use: in_use,
        weapon_class,
        states: Vec::new(),
    })
}

/// The weapon class of a loaded player (the player branch; no states, so no
/// disguise remap applies, and a loaded player is not in mode 0 or 0x11), as
/// a `wclass` index. The hand
/// item is body slot 4 when it passes the usable test
/// (neither flag 0x100 nor 0x4000; identification is not tested) and its
/// `component` is 5 or 6; else body slot 5 when that passes. No hand item
/// or one whose `component` is not 5 or 6 → 0 (the class's `hth`).
/// Otherwise its `wclass` index. The class pairs (`1js`…`1ss` of a
/// barbarian holding two weapons, `ht2` of an assassin holding two claws)
/// and the two-handed column never yield index 1 or 7, the only ones the
/// appearance fill reads: every `bow` and `xbw` row of the `weapons` table has
/// both columns equal.
fn hand_weapon_class(
    body_grid: &[Option<usize>; BODY_SLOTS],
    locs: &[(u32, u8, u32, usize)],
    a: &AppearanceTables,
) -> i32 {
    use d2_sim::items::inventory::{body, iflag};
    let passes = |k: usize| locs[k].2 & (iflag::BROKEN | iflag::F4000) == 0;
    let hand = |k: usize| {
        a.items
            .get(locs[k].3)
            .filter(|g| matches!(g.component, 5 | 6))
    };
    let right = body_grid[usize::from(body::RIGHT_HAND)].filter(|&k| passes(k));
    let item = match right.filter(|&k| hand(k).is_some()) {
        Some(k) => Some(k),
        None => body_grid[usize::from(body::LEFT_HAND)].filter(|&k| passes(k)),
    };
    item.and_then(hand)
        .map_or(0, |g| appearance::wclass_index(g.wclass))
}

/// Max sockets of the record at item level `ilvl`: the item's gem sockets,
/// capped by the item type's limit for that item level band.
fn max_sockets(t: &ItemTables, record: usize, ilvl: i32) -> i32 {
    let (Some(r), Some(it)) = (t.item(record), t.itype_of(record)) else {
        return 0;
    };
    let m = match ilvl.max(1) {
        ..=25 => it.maxsock1,
        26..=40 => it.maxsock25,
        _ => it.maxsock40,
    };
    i32::from(r.gemsockets.min(m))
}

impl<H: LifecycleHooks, R: InvRest + ?Sized> SaveItems for InvDesk<'_, '_, H, R> {
    fn items(&self, owner: UnitId) -> Vec<UnitId> {
        self.state.items_of(owner)
    }
    fn body_item(&self, owner: UnitId, loc: u8) -> Option<UnitId> {
        self.state.of(owner)?.body_item(loc)
    }
    fn weapon_in_use(&self, owner: UnitId) -> Option<UnitId> {
        let g = self.state.of(owner)?.weapon_guid;
        self.state
            .items_of(owner)
            .into_iter()
            .find(|u| self.state.items.get(u).is_some_and(|d| d.guid == g))
    }
    fn cursor(&self, owner: UnitId) -> Option<UnitId> {
        self.state.cursor_of(owner)
    }
    fn node(&self, item: UnitId) -> Option<(u8, u8)> {
        self.state
            .items
            .get(&item)
            .map(|d| (d.node_kind, d.body_loc))
    }
    fn flags2(&self, item: UnitId) -> u32 {
        self.econ.units.get(item).map_or(0, |r| r.flags2)
    }
    fn stream(&self, item: UnitId) -> Option<StreamItem> {
        let d = self.state.items.get(&item)?;
        let mut s = self.stream_item(d.guid, 0, d.page)?;
        // The save-only 32 bits are the unit record's init seed
        // (as `InvDesk::save_view`).
        s.unit28 = self.econ.units.get(item)?.init_seed;
        Some(s)
    }
    fn write_back(&self, item: UnitId, wb: WriteBack) {
        self.queue_write_back(item, wb);
    }
    fn appearance(&self, item: UnitId) -> Option<EquippedItem> {
        let d = self.state.items.get(&item)?;
        let it = self.econ.items.get(item)?;
        Some(EquippedItem {
            record: it.record,
            mode: d.mode,
            body_loc: d.body_loc,
            quality: it.quality,
            prefix: it.prefix,
            suffix: it.suffix,
            auto_affix: it.auto_affix,
            file_index: it.file_index,
            flags: it.flags,
            max_sockets: max_sockets(self.econ.tables, it.record, it.ilvl),
            first_child: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::d2s::appearance::{ColourTables, IsA, ItemGfx, ReferenceSlots};
    use d2_sim::items::inventory::iflag;

    fn tables() -> AppearanceTables {
        let gfx = |component, wclass: &[u8; 4]| ItemGfx {
            component,
            wclass: *wclass,
            ..ItemGfx::default()
        };
        // 0 crossbow, 1 bolts quiver, 2 sword.
        let items = vec![gfx(5, b"xbw "), gfx(16, b"\0\0\0\0"), gfx(5, b"1hs ")];
        let types = IsA::default();
        let reference = ReferenceSlots::game(&types);
        AppearanceTables::new(
            items,
            Vec::new(),
            types,
            ColourTables::default(),
            &reference,
        )
    }

    /// Slot 4 first, slot 5 when 4 fails; no identified
    /// test; quivers (component 16) give no class.
    #[test]
    fn hand_weapon_class_slots() {
        let a = tables();
        let mut grid = [None; BODY_SLOTS];
        grid[4] = Some(0);
        grid[5] = Some(1);
        // Unidentified crossbow (no 0x10): still `xbw`.
        let locs = [(1, 4, 0, 0), (1, 5, 0, 1)];
        assert_eq!(hand_weapon_class(&grid, &locs, &a), 7);
        // Broken crossbow in slot 4 → slot 5's quiver → `hth`.
        let locs = [(1, 4, iflag::BROKEN, 0), (1, 5, 0, 1)];
        assert_eq!(hand_weapon_class(&grid, &locs, &a), 0);
        // Quiver in slot 4 → slot 5's crossbow.
        grid[4] = Some(1);
        grid[5] = Some(0);
        let locs = [(1, 5, 0x10, 0), (1, 4, 0, 1)];
        assert_eq!(hand_weapon_class(&grid, &locs, &a), 7);
        // Slot 4 sword passes first.
        grid[4] = Some(1);
        grid[5] = Some(0);
        let locs = [(1, 5, 0, 0), (1, 4, 0, 2)];
        assert_eq!(hand_weapon_class(&grid, &locs, &a), 2);
    }
}
