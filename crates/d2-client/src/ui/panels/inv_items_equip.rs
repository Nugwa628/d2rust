//! A left press on one of the local player's equipment boxes: the box hit, the
//! use cursor, the equip check run on the client model and the C→S message of
//! its result, or the refusal speech when the check refuses the cursor item.
//!
//! Is d2-sim's ([`equip_check`]) over [`ClientInv`], a read-only
//! view of the model: the local player's worn, page-0 and cursor items
//! as the last 0x9C / 0x9D streams describe them, the player's received
//! stats and the item tables. The client changes nothing locally; the
//! server decides the move.
//!
//! Not modelled (no client state for these yet):
//! - the send throttle is taken as open, and the busy test
//!   sees only the marks of the same press (a marked item
//!   is not ready for the rest of the press);
//! - "sound of i" and the lift flag
//!   (audio deferred; no reader of the flag in the preview);
//! - the transmogrify cursor (state 8, 0x4C) and the store click S(i):
//!   the inventory panel runs in mode m = 0 (own-body mode), so S is
//!   never reached.

use d2_proto::client::{
    RemoveBodyItem, StackItems, Swap1HWith2H, Swap2HandedItem, SwapCursorWithBody,
};
use d2_proto::item_bits::{hflag, ItemBits};
use d2_sim::items::inventory::equip::{
    equip_check, one_or_two_handed_for, other_hand, res, CLASS_NONE,
};
use d2_sim::items::inventory::grid::{find_free_position, place_at_body, place_at_page};
use d2_sim::items::inventory::{
    level_requirement, AffixReq, InteractionTarget, InvItem, InvTables, InvWorld, Inventory,
    LevelReqItem, LevelReqUnit, UnitKind, NO_GUID,
};
use d2_sim::items::ItemTables;
use d2_sim::units::UnitId;

use crate::bridge::items::{self, mode, ItemView};
use crate::bridge::world::ClientWorld;
use crate::ui::panel::ClientIntent;
use crate::ui::panels::PanelOutput;

/// Player events of the equip pre-check.
pub mod event {
    /// `impossible`.
    pub const IMPOSSIBLE: u16 = 19;
    /// `cantuseyet`.
    pub const CANT_USE_YET: u16 = 20;
}

/// Unit flag-ex 0x2000000: expansion.
const FLAG_EX_EXPANSION: u32 = 0x0200_0000;
/// Item stats read here.
const STAT_QUANTITY: u16 = 70;
const STAT_REQ_PERCENT: u16 = 91;
const STAT_LEVELREQ: u16 = 92;
const STAT_NONCLASSSKILL: u16 = 97;
const STAT_SINGLESKILL: u16 = 107;
const STAT_SOCKETS: u16 = 194;

/// The local player.
const PLAYER: UnitId = UnitId(0);

/// One item of the view: its inventory record and its decoded stream.
struct Entry {
    item: InvItem,
    bits: Option<ItemBits>,
    /// Its socket fillers (mode 6 items owned by it), as view indices.
    fillers: Vec<usize>,
}

/// The client model as the inventory rules read it: the local
/// player and its items (module doc).
pub struct ClientInv<'a> {
    t: &'a InvTables,
    it: &'a ItemTables,
    expansion: bool,
    class: u8,
    flag_ex: u32,
    stats: [i32; 3],
    entries: Vec<Entry>,
}

fn unit_of(i: usize) -> UnitId {
    UnitId(i as u32 + 1)
}

impl<'a> ClientInv<'a> {
    /// The view of `world`'s local player and its inventory: the worn
    /// items at their body locations, the page-0 items in their cells,
    /// the cursor item. `None` without a local player.
    pub fn build(
        world: &ClientWorld,
        t: &'a InvTables,
        it: &'a ItemTables,
        decode: &dyn Fn(&[u8]) -> Option<ItemBits>,
    ) -> Option<(Self, Inventory, Vec<ItemView>)> {
        let me = world.local()?;
        let class = me.class as u8;
        let views: Vec<ItemView> = items::items(world)
            .into_iter()
            .filter(|i| !i.on_ground() && !i.store)
            .collect();
        let mut v = ClientInv {
            t,
            it,
            expansion: world.expansion != 0,
            class,
            flag_ex: me.flag_ex,
            stats: [me.stat(0), me.stat(2), me.stat(12)],
            entries: Vec::with_capacity(views.len()),
        };
        for iv in &views {
            let record = iv
                .code
                .and_then(|c| t.items.iter().position(|r| r.code == c));
            let mut item = InvItem::new(iv.key.guid, record.unwrap_or(usize::MAX));
            item.flags = iv.flags;
            item.mode = iv.mode;
            v.entries.push(Entry {
                item,
                bits: items::stream(world, iv.key).and_then(decode),
                fillers: Vec::new(),
            });
        }
        for (i, iv) in views.iter().enumerate() {
            if iv.mode != mode::SOCKETED {
                continue;
            }
            if let Some(o) = views.iter().position(|o| Some(o.key) == iv.owner) {
                v.entries[o].fillers.push(i);
            }
        }
        let mut inv = Inventory::new(PLAYER, UnitKind::Player { class }, me.key.guid);
        for (i, iv) in views.iter().enumerate() {
            if iv.owner != Some(me.key) {
                continue;
            }
            let u = unit_of(i);
            match iv.mode {
                mode::BODY => {
                    place_at_body(&mut inv, &mut v, u, iv.body);
                    if let Some(d) = v.item_mut(u) {
                        d.body_loc = iv.body;
                        d.mode = mode::BODY;
                    }
                }
                mode::STORED if iv.page == 0 => {
                    place_at_page(&mut inv, &mut v, t, u, 0, i32::from(iv.x), i32::from(iv.y));
                    if let Some(d) = v.item_mut(u) {
                        d.mode = mode::STORED;
                    }
                }
                _ => {}
            }
        }
        if let Some(c) = items::cursor_item(world) {
            if let Some(i) = views.iter().position(|o| o.key == c.key) {
                inv.set_cursor(Some(unit_of(i)));
            }
        }
        Some((v, inv, views))
    }

    /// The item's row in the game's item tables (by code).
    fn it_record(&self, d: &InvItem) -> Option<usize> {
        self.it.find_code(self.t.item(d.record)?.code)
    }

    fn entry(&self, u: UnitId) -> Option<&Entry> {
        usize::try_from(u.0)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| self.entries.get(i))
    }

    /// The main list and the runeword list of the item (the item bits'
    /// C = −1 and, with a runeword, c = L − 1); the set lists
    /// are partial bonuses and not the item's own stats.
    fn own_stats(&self, u: UnitId) -> impl Iterator<Item = &d2_proto::item_bits::Stat> {
        let lists = self.entry(u).and_then(|e| e.bits.as_ref()).map(|b| {
            let main = b.lists.first().and_then(Option::as_ref);
            let rw = (b.flags & hflag::RUNEWORD != 0 && b.lists.len() > 1)
                .then(|| b.lists.last().and_then(Option::as_ref))
                .flatten();
            main.into_iter().chain(rw).flatten()
        });
        lists.into_iter().flatten()
    }

    fn sum_stat(&self, u: UnitId, s: u16) -> i32 {
        self.own_stats(u)
            .filter(|e| e.stat == s)
            .map(|e| e.value() as i32)
            .sum()
    }

    fn level_req_item(&self, u: UnitId) -> LevelReqItem {
        let Some((e, b)) = self.entry(u).and_then(|e| Some((e, e.bits.as_ref()?))) else {
            return LevelReqItem {
                quality: 2,
                ..LevelReqItem::default()
            };
        };
        let it = self.it;
        // Item data ids from the wire ids.
        let p = it.n_suffix as u16;
        let aff = |id: u16| {
            d2_sim::items::affixes::affix(it, id).map(|a| AffixReq {
                levelreq: a.levelreq,
                class: a.class,
                classlevelreq: a.classlevelreq,
            })
        };
        let pre = |w: u16| aff(if w == 0 { 0 } else { w + p });
        let q = &b.quality_fields;
        let mut prefixes = [None; 3];
        let mut suffixes = [None; 3];
        if let Some((pf, sf)) = q.magic {
            prefixes[0] = pre(pf);
            suffixes[0] = aff(sf);
        }
        if let Some(slots) = q.rare_slots {
            for (i, (pf, sf)) in slots.into_iter().enumerate() {
                prefixes[i] = pre(pf);
                suffixes[i] = aff(sf);
            }
        }
        let auto = b
            .auto_affix
            .filter(|&a| a != 0)
            .and_then(|a| aff(a + it.first_auto() as u16));
        let row = q.file_index.map(|f| f as usize);
        let skills = |s: u16| {
            self.own_stats(u)
                .filter(move |e| e.stat == s)
                .filter_map(|e| it.skills.get(e.param as usize))
                .collect::<Vec<_>>()
        };
        LevelReqItem {
            quality: b.quality,
            automagic: auto,
            prefixes,
            suffixes,
            set_lvlreq: row
                .and_then(|i| it.setitems.get(i))
                .map_or(0, |s| s.lvl_req),
            unique_lvlreq: row.and_then(|i| it.uniques.get(i)).map(|r| r.lvl_req),
            version: b.version,
            class_levelreq: self
                .t
                .item(e.item.record)
                .map_or(0, |r| i32::from(r.levelreq)),
            fillers: e
                .fillers
                .iter()
                .map(|&f| self.level_req_item(unit_of(f)))
                .collect(),
            single_skills: skills(STAT_SINGLESKILL)
                .into_iter()
                .map(|s| s.reqlevel)
                .collect(),
            nonclass_skills: skills(STAT_NONCLASSSKILL)
                .into_iter()
                .map(|s| (s.reqlevel, i32::from(s.charclass)))
                .collect(),
            stat_levelreq: self.sum_stat(u, STAT_LEVELREQ),
        }
    }
}

impl InvWorld for ClientInv<'_> {
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn item(&self, item: UnitId) -> Option<&InvItem> {
        self.entry(item).map(|e| &e.item)
    }
    fn item_mut(&mut self, item: UnitId) -> Option<&mut InvItem> {
        let i = usize::try_from(item.0).ok()?.checked_sub(1)?;
        self.entries.get_mut(i).map(|e| &mut e.item)
    }
    fn item_by_guid(&self, guid: u32) -> Option<UnitId> {
        self.entries
            .iter()
            .position(|e| e.item.guid == guid)
            .map(unit_of)
    }
    fn item_stat(&self, item: UnitId, stat: u16) -> i32 {
        match stat {
            STAT_QUANTITY => self
                .entry(item)
                .and_then(|e| e.bits.as_ref()?.quantity)
                .map_or(0, i32::from),
            STAT_SOCKETS => self
                .entry(item)
                .and_then(|e| e.bits.as_ref()?.sockets)
                .map_or(0, i32::from),
            s => self.sum_stat(item, s),
        }
    }
    // The view is only read by the body press; the placements that build it need
    // none of these.
    fn unlink_from(&mut self, _: UnitId, _: UnitId) {}
    fn remove_from_room(&mut self, _: UnitId) {}
    fn clear_targetable(&mut self, _: UnitId) {}
    fn link_check(&mut self, _: UnitId, _: UnitId, _: u8) -> bool {
        true
    }
    fn charm_relink(&mut self, _: UnitId, _: UnitId) {}
    fn active_item(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn stat_refresh(&mut self, _: UnitId) {}
    fn socket_filled(&self, item: UnitId) -> bool {
        self.entry(item).is_some_and(|e| !e.fillers.is_empty())
    }
    fn owner_refresh(&mut self, _: UnitId) {}
    fn inventory_pass(&mut self, _: UnitId) {}
    fn trade_hook(&mut self, _: UnitId, _: UnitId) {}
    fn weapon_in_use_update(&mut self, _: UnitId) {}
    fn stat_link(&mut self, _: UnitId, _: UnitId) {}
    fn weapon_bookkeeping(&mut self, _: UnitId, _: UnitId) {}

    fn unit_kind(&self, unit: UnitId) -> Option<UnitKind> {
        if unit == PLAYER {
            Some(UnitKind::Player { class: self.class })
        } else {
            self.entry(unit).map(|_| UnitKind::Item)
        }
    }
    /// The player's received stats (strength 0, dexterity 2, level 12).
    fn unit_stat(&self, unit: UnitId, stat: u16) -> i32 {
        if unit != PLAYER {
            return 0;
        }
        match stat {
            0 => self.stats[0],
            2 => self.stats[1],
            12 => self.stats[2],
            _ => 0,
        }
    }
    fn req_percent(&self, item: UnitId) -> i32 {
        self.sum_stat(item, STAT_REQ_PERCENT)
    }
    /// A worn item whose stats are on (item flag 0x4000 clear) has
    /// its list linked to the wearer.
    fn item_active_on(&self, item: UnitId, unit: UnitId) -> bool {
        unit == PLAYER
            && self
                .item(item)
                .is_some_and(|d| d.mode == mode::BODY && d.flags & 0x4000 == 0)
    }
    /// The socket contribution: the fillers'
    /// stat, only for an item with `hasinv` worn in mode 1.
    fn own_contribution(&self, item: UnitId, _: UnitId, stat: u16) -> i32 {
        let Some(e) = self.entry(item) else { return 0 };
        let hasinv = self
            .it_record(&e.item)
            .and_then(|r| self.it.item(r))
            .is_some_and(|r| r.hasinv != 0);
        if !hasinv || e.item.mode != mode::BODY {
            return 0;
        }
        e.fillers
            .iter()
            .map(|&f| self.sum_stat(unit_of(f), stat))
            .sum()
    }
    fn level_requirement(&self, item: UnitId, _: UnitId) -> i32 {
        let who = LevelReqUnit {
            class: u32::from(self.class),
            player: true,
            expansion: self.flag_ex & FLAG_EX_EXPANSION != 0,
        };
        level_requirement(&self.level_req_item(item), Some(&who))
    }
    fn two_handed(&self, item: UnitId) -> bool {
        self.item(item)
            .and_then(|d| self.t.item(d.record))
            .is_some_and(|r| r.twohanded != 0)
    }
    fn one_or_two_handed(&self, unit: UnitId, item: UnitId) -> bool {
        self.item(item)
            .is_some_and(|d| one_or_two_handed_for(self.t, d.record, self.unit_kind(unit)))
    }
    /// The primary type's itemtypes `shoots`.
    fn ammo_type(&self, item: UnitId) -> Option<i16> {
        let r = self.it_record(self.item(item)?)?;
        self.it.itype_of(r).map(|t| t.shoots as i16)
    }
    fn fits_free_page0(&self, inv: &Inventory, item: UnitId) -> bool {
        let mut copy = inv.clone();
        find_free_position(&mut copy, self, self.t, item, 0).is_some()
    }
    fn quality(&self, item: UnitId) -> u8 {
        self.entry(item)
            .and_then(|e| e.bits.as_ref())
            .map_or(0, |b| b.quality)
    }
    fn stack_file_index(&self, item: UnitId) -> i32 {
        self.entry(item)
            .and_then(|e| e.bits.as_ref()?.quality_fields.file_index)
            .map_or(-1, |f| f as i32)
    }
    fn has_sockets(&self, item: UnitId) -> bool {
        self.item_stat(item, STAT_SOCKETS) != 0
    }
    fn has_allowed_location(&self, _: UnitId) -> bool {
        false
    }
    fn quiver_kind(&self, _: UnitId) -> bool {
        false
    }
    fn targeting_probe(&self, _: UnitId) -> u32 {
        6
    }
    fn queue_untarget(&mut self, _: UnitId, _: u32) {}
    fn interaction(&self, _: UnitId) -> InteractionTarget {
        InteractionTarget::None
    }
    fn clear_interaction(&mut self, _: UnitId) {}
    fn player_data_4c(&self, _: UnitId) -> u32 {
        0
    }
    fn player_data_50(&self, _: UnitId) -> u32 {
        0
    }
    fn npc_talking(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
    fn player_trade_gate(&self, _: UnitId) -> Option<bool> {
        None
    }
    fn same_act(&self, _: UnitId, _: UnitId) -> bool {
        true
    }
    fn within_range(&self, _: UnitId, _: UnitId, _: i32) -> bool {
        true
    }
}

/// The cursor state of the press: 6 with a used item
/// (identify), else 4 with a cursor item, else 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Plain,
    Item,
    /// State 6 with the used item's GUID.
    Use(u32),
}

/// What a body press produced: the outputs, and whether the use cursor
/// ends (rule 2 sent 0x27, or rule 1.4's cancel).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BodyPress {
    pub out: Vec<PanelOutput>,
    pub end_use: bool,
}

fn intent<M: d2_proto::FixedMessage>(m: &M) -> PanelOutput {
    PanelOutput::Intent(ClientIntent::from_message(m))
}

/// The body press rules for location `loc` (1–10)
/// after the socket test: `inv` / `w` the view of the model,
/// `cursor` the cursor state.
pub fn body_press(w: &ClientInv, inv: &Inventory, loc: u8, cursor: Cursor) -> BodyPress {
    let mut p = BodyPress::default();
    let mut marked: Vec<UnitId> = Vec::new();
    // Ready(i…): not busy for each (here: not marked in this
    // press); the send throttle is open (module doc).
    let ready = |marked: &[UnitId], is: &[Option<UnitId>]| {
        is.iter().all(|i| i.is_some_and(|i| !marked.contains(&i)))
    };
    let guid = |i: UnitId| w.item(i).map_or(NO_GUID, |d| d.guid);
    let hands = matches!(loc, 4 | 5);
    let t_item = inv.body_item(loc);
    let o = other_hand(loc).unwrap_or(0);
    let x_item = other_hand(loc).and_then(|o| inv.body_item(o));
    let c = inv.cursor();
    let mut r = false;
    // Rule 2: the use cursor.
    if let Cursor::Use(u) = cursor {
        if let Some(t) = t_item.filter(|&t| ready(&marked, &[Some(t)])) {
            p.out.push(intent(&d2_proto::client::UseItemAction {
                target: guid(t),
                used: u,
            }));
            p.end_use = true;
            marked.push(t);
            r = true;
        }
        if !hands {
            if !r {
                p.end_use = true;
            }
            return p;
        }
    }
    // Rule 3.
    let e = equip_check(inv, w, w.t, PLAYER, loc, c, false);
    let bodyloc = loc;
    match e {
        res::NO => {
            if let Some(c) = c {
                // Pre-check: only the requirements failed and the
                // item's class is none or the player's → `cantuseyet`.
                let class = w
                    .item(c)
                    .and_then(|d| w.t.itype_of(d.record))
                    .map_or(CLASS_NONE, |r| r.class);
                let only_req = equip_check(inv, w, w.t, PLAYER, loc, Some(c), true) != res::NO;
                let ev = if only_req && (class == CLASS_NONE || class == w.class) {
                    event::CANT_USE_YET
                } else {
                    event::IMPOSSIBLE
                };
                p.out.push(PanelOutput::PlayerEvent(ev));
            }
            r = true;
        }
        res::FREE => {
            if ready(&marked, &[c]) {
                let c = c.unwrap_or(PLAYER);
                p.out.push(intent(&d2_proto::client::EquipItem {
                    item: guid(c),
                    bodyloc,
                }));
                marked.push(c);
                r = true;
            }
        }
        res::OTHER_HAND_BLOCKS if hands => {
            if ready(&marked, &[c, x_item]) {
                let c = c.unwrap_or(PLAYER);
                p.out.push(intent(&Swap2HandedItem {
                    item: guid(c),
                    bodyloc,
                }));
                marked.extend(x_item);
                marked.push(c);
                r = true;
            }
        }
        res::REMOVABLE => {
            // Own-body mode (m = 0): the hands test Ready(T), the other
            // locations only the throttle.
            if !hands || ready(&marked, &[t_item]) {
                p.out.push(intent(&RemoveBodyItem {
                    bodyloc: u16::from(loc),
                }));
                if hands {
                    marked.extend(t_item);
                }
                r = true;
            }
        }
        res::OTHER_HAND_TWO_HANDED if hands => {
            if ready(&marked, &[x_item]) {
                p.out.push(intent(&RemoveBodyItem {
                    bodyloc: u16::from(o),
                }));
                marked.extend(x_item);
                r = true;
            }
        }
        res::SWAP => {
            // The hands test Ready(T), the other locations Ready(c).
            let probe = if hands { t_item } else { c };
            if ready(&marked, &[probe]) {
                let c = c.unwrap_or(PLAYER);
                p.out.push(intent(&SwapCursorWithBody {
                    item: guid(c),
                    bodyloc,
                }));
                marked.push(c);
                r = true;
            }
        }
        res::STACK if hands => {
            if ready(&marked, &[c]) {
                let (c, t) = (c.unwrap_or(PLAYER), t_item.unwrap_or(PLAYER));
                p.out.push(intent(&StackItems {
                    src: guid(c),
                    dst: guid(t),
                }));
                marked.push(c);
                r = true;
            }
        }
        // The last send of the press: no mark is read after it.
        res::SWAP_OTHER_TO_PAGE if hands && ready(&marked, &[c, t_item, x_item]) => {
            let c = c.unwrap_or(PLAYER);
            p.out.push(intent(&Swap1HWith2H {
                item: guid(c),
                bodyloc,
            }));
            r = true;
        }
        // "—": the result stays as it was.
        _ => {}
    }
    // Rule 1.4: r = 0 under the use cursor cancels it.
    if !r && matches!(cursor, Cursor::Use(_)) {
        p.end_use = true;
    }
    p
}

#[cfg(test)]
#[path = "inv_items_equip_tests.rs"]
pub(crate) mod tests;
