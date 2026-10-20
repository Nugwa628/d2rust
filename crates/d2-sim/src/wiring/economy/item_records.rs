//! An item unit made from one save record read back
//! (`items::bitstream::read`): allocation of the unit at the record's mode
//! (with a new GUID), the decode into it, then item flag 0x80000 set, 0x2000
//! cleared and the replenish timers scheduled. Used by the item copy and the
//! save load.
//!
//! What the record carries goes where the writer read it from: the item data
//! to the item store, the save-only 32 bits to the item seed, the base values
//! (defense, max durability, sockets) and the totals the writer sent
//! (durability, gold, quantity, quest difficulty) as base stats, the property
//! lists to their keyed lists. The record's position, body location and page
//! are the item data of the inventory state (`wiring::inventory`): the caller
//! places them.

use std::cell::RefCell;

use super::item_stats::{StatCtx, UnitStats};
use super::{Economy, EconomyError};
use crate::items::bitstream::read::ReadItem;
use crate::items::bitstream::{hflag, StatEntry, RUNEWORD_STATE, SET_STATES};
use crate::items::create::max_sockets_at;
use crate::items::tables::ItemRec;
use crate::items::{flag, replenish_timer_at, stat, ty, Item, ItemStats, ListKey};
use crate::rng::Seed;
use crate::units::hooks::Sim;
use crate::units::lifecycle::{self, AllocRequest, LifecycleHooks};
use crate::units::{RoomId, UnitId, UnitType};

/// Stat 57 `poisonmindam`.
const POISONMINDAM: u16 = 57;
/// Item timer event 3: replenish.
const EVENT_REPLENISH: u32 = 3;
/// Flags of a set list read back.
const SET_FLAGS: u32 = 0x2040;
/// Flags of the runeword list (state 171, flag 0x40).
const RUNEWORD_FLAGS: u32 = 0x40;

impl<H: LifecycleHooks> Economy<'_, H> {
    /// Builds an item unit from a record read back: allocates an item unit of
    /// the record's class in `room` at the record's mode, decodes the record
    /// into it, sets item flag 0x80000 and clears 0x2000, and schedules the
    /// replenish event. No item-roll draw: the record holds the item's
    /// fields.
    pub fn item_from_record(
        &mut self,
        rec: &ReadItem,
        room: Option<RoomId>,
    ) -> Result<UnitId, EconomyError> {
        let it = &rec.item;
        let req = AllocRequest {
            ty: UnitType::Item,
            class: rec.record as u32,
            room,
            add: true,
            fixed_guid: None,
            mode: it.mode,
            allied: false,
        };
        let unit = {
            let mut sim = Sim {
                game: &mut *self.game,
                units: &mut *self.units,
                stats: &mut *self.stats,
                data: self.data,
            };
            lifecycle::allocate(&mut sim, &mut *self.hooks, &mut self.fields.seed, &req)?
        }
        .ok_or(EconomyError::NotAllocated)?;
        let mut item = Item::new(rec.record, it.version, ());
        item.ilvl = it.ilvl;
        item.quality = it.quality;
        // The ear record's class is its file index.
        item.file_index = if it.flags & hflag::EAR != 0 {
            it.ear_class
        } else {
            it.file_index
        };
        item.prefix = it.prefix;
        // The loader stores the runeword's 16-bit name id in the first
        // prefix word, also for a runeword the sockets no longer back.
        if it.flags & hflag::RUNEWORD != 0 {
            item.prefix[0] = it.runeword;
        }
        item.suffix = it.suffix;
        item.rare_prefix = it.rare_prefix;
        item.rare_suffix = it.rare_suffix;
        item.auto_affix = it.auto_affix;
        item.inv_page = it.page;
        item.gfx = it.gfx;
        item.name = it.name;
        item.ear_level = it.ear_level;
        // The two words a and b of the save trailer go to the item's realm
        // data.
        if let Some((a, b)) = it.save_trailer {
            item.realm_data = [a, b];
        }
        item.flags = (it.flags | flag::INIT) & !flag::INSTORE;
        let r = self
            .units
            .get_mut(unit)
            .ok_or(EconomyError::NoRecord(unit))?;
        // Item seed from a save: the full record's 32 bits v; the compact
        // reader sets 0. The unit's init seed := v and its unit seed :=
        // `init_low(v)` = {v, 666}. The item data seed and the start seed
        // keep the allocation's game-seed step.
        let v = if it.compact || it.alt { 0 } else { it.unit28 };
        r.init_seed = v;
        r.seed = Seed::init_low(v);
        if let Some((s, start)) = r.item_seed {
            item.item_seed = s;
            item.start_seed = start;
        }
        let frame = self.game.frame as u32;
        let event3_at = {
            let ctx = RefCell::new(StatCtx::new(self.stats, self.hooks));
            let mut s = UnitStats::new(&ctx, unit);
            let t = self.tables;
            let rebuild = t.item(rec.record).map(|r| Rebuild {
                rec: r.clone(),
                weapon: t.is_type(rec.record, ty::WEAP as i16),
                armor: t.is_type(rec.record, ty::ARMO as i16),
                throwable: t.itype_of(rec.record).is_some_and(|y| y.throwable != 0),
                max_sockets: max_sockets_at(t, rec.record, it.ilvl),
            });
            set_record_stats(&mut s, it, rebuild.as_ref());
            replenish_timer_at(&s, false, frame, self.game.tick_rate)
        };
        self.items.insert(unit, item);
        if let Some(f) = event3_at {
            // TODO: the event's arguments are not settled; scheduled with
            // 0, 0 (as at creation).
            self.game
                .schedule_event(unit, EVENT_REPLENISH, f as i32, None, 0, 0)?;
        }
        Ok(unit)
    }

    /// Sets the ground expiry frame of an item just placed on the ground:
    /// quest items 0 (never);
    /// else the frame plus 30,000 (magic, or a gem / rune / jewel), 45,000
    /// (quality 5-9, or gold over 10,000) or 15,000.
    pub fn set_ground_expiry(&mut self, item: UnitId) {
        let Some(it) = self.items.get(item) else {
            return;
        };
        let (record, quality) = (it.record, it.quality);
        let quest = self.tables.item(record).is_some_and(|r| r.quest != 0);
        let expiry = if quest {
            0
        } else {
            let gold = self.tables.is_type(record, ty::GOLD as i16)
                && self.stats.unit_total(item, stat::GOLD, 0) > 10000;
            let filler = [ty::GEM, ty::RUNE, ty::JEWL]
                .into_iter()
                .any(|t| self.tables.is_type(record, t as i16));
            let add = if quality == 4 {
                30000
            } else if (5..=9).contains(&quality) || gold {
                45000
            } else if filler {
                30000
            } else {
                15000
            };
            self.game.after(add)
        };
        if let Some(it) = self.items.get_mut(item) {
            it.ground_expiry = expiry;
        }
    }
}

/// The items-table facts the decoder rebuilds base stats from.
struct Rebuild {
    rec: ItemRec,
    weapon: bool,
    armor: bool,
    throwable: bool,
    /// Max sockets at the record's item level.
    max_sockets: i32,
}

/// The weapon's base speed and damage as the decoder rebuilds them: the items
/// columns, ⌊3v / 4⌋ with floors for quality 1,
/// × 3 / 2 for item flag 0x400000.
fn rebuild_weapon(s: &mut impl ItemStats, rb: &Rebuild, quality: u8, flags: u32) {
    let r = &rb.rec;
    s.set_base(stat::ATTACKRATE, 0, r.speed.wrapping_neg());
    let missile = r.maxmisdam != 0;
    let mut six = [
        (stat::MAXDAMAGE, i32::from(r.maxdam), true, 2),
        (stat::MINDAMAGE, i32::from(r.mindam), true, 1),
        (stat::SECONDARY_MAXDAMAGE, i32::from(r.maxdam2), true, 2),
        (stat::SECONDARY_MINDAMAGE, i32::from(r.mindam2), true, 1),
        (stat::THROW_MINDAMAGE, i32::from(r.minmisdam), missile, 1),
        (stat::THROW_MAXDAMAGE, i32::from(r.maxmisdam), missile, 2),
    ];
    for (id, v, set, floor) in &mut six {
        if !*set {
            continue;
        }
        if quality == 1 {
            *v = (v.wrapping_mul(3) / 4).max(*floor);
        }
        if flags & flag::ETHEREAL != 0 {
            *v = v.wrapping_mul(3) / 2;
        }
        s.set_base(*id, 0, *v);
    }
}

/// An entry for stat 17 raises the base maximum damages,
/// one for stat 18 the base minimum damages, to the items column when
/// below it (the throw damage only for a throwable item).
fn raise_damage(s: &mut impl ItemStats, rb: &Rebuild, max: bool) {
    let r = &rb.rec;
    let cols = if max {
        [
            (stat::MAXDAMAGE, r.maxdam, true),
            (stat::SECONDARY_MAXDAMAGE, r.maxdam2, true),
            (stat::THROW_MAXDAMAGE, r.maxmisdam, rb.throwable),
        ]
    } else {
        [
            (stat::MINDAMAGE, r.mindam, true),
            (stat::SECONDARY_MINDAMAGE, r.mindam2, true),
            (stat::THROW_MINDAMAGE, r.minmisdam, rb.throwable),
        ]
    };
    for (id, col, go) in cols {
        let col = i32::from(col);
        if go && s.base(id, 0) < col {
            s.set_base(id, 0, col);
        }
    }
}

/// The values the record carries, where the writer read them.
fn set_record_stats(
    s: &mut impl ItemStats,
    it: &crate::items::bitstream::StreamItem,
    rebuild: Option<&Rebuild>,
) {
    if it.compact {
        if it.kind.gold {
            s.set_base(stat::GOLD, 0, it.total_gold);
        }
        if it.quest_diff {
            s.set_base(stat::QUESTITEMDIFFICULTY, 0, it.total_quest_diff);
        }
        return;
    }
    if it.alt {
        return;
    }
    if it.kind.armor {
        s.set_base(stat::ARMORCLASS, 0, it.base_defense);
    }
    if it.kind.armor || it.kind.weapon {
        s.set_base(stat::MAXDURABILITY, 0, it.base_max_dur);
        if it.base_max_dur != 0 {
            s.set_base(stat::DURABILITY, 0, it.total_dur);
        }
    } else if it.kind.gold {
        s.set_base(stat::GOLD, 0, it.total_gold);
    }
    if it.stackable {
        s.set_base(stat::QUANTITY, 0, it.total_quantity);
    }
    // Socket setter: count := min(max(v, 1), min(w × h, 6), max sockets);
    // w × h = 0 sets nothing (flag 0x800 is already the record's).
    if it.flags & hflag::SOCKETED != 0 {
        if let Some(rb) = rebuild {
            let cells = i32::from(rb.rec.invwidth) * i32::from(rb.rec.invheight);
            if cells != 0 {
                let n = it.base_sockets.max(1).min(cells.min(6)).min(rb.max_sockets);
                s.set_base(stat::NUMSOCKETS, 0, n);
            }
        }
    }
    // The base values the stream does not carry.
    if let Some(rb) = rebuild {
        if rb.weapon {
            rebuild_weapon(s, rb, it.quality, it.flags);
        } else if rb.armor {
            s.set_base(stat::TOBLOCK, 0, i32::from(rb.rec.block));
            s.set_base(stat::VELOCITYPERCENT, 0, rb.rec.speed.wrapping_neg());
        }
    }
    let mut put = |key: ListKey, list: &Option<Vec<StatEntry>>| {
        for e in list.iter().flatten() {
            if let Some(rb) = rebuild {
                // Raise the base damages before the entry is stored.
                match e.stat {
                    stat::MAXDAMAGE_PERCENT => raise_damage(s, rb, true),
                    stat::MINDAMAGE_PERCENT => raise_damage(s, rb, false),
                    _ => {}
                }
            }
            s.list_set(key, e.stat, e.param, e.value);
            // An entry for stat 57 (read with 58, 59) sets `poison_count` to
            // 1 in that list.
            if e.stat == POISONMINDAM {
                s.list_set(key, stat::POISON_COUNT, 0, 1);
            }
        }
    };
    put(ListKey::ITEM, &it.main);
    for (i, list) in it.sets.iter().enumerate() {
        put(
            ListKey {
                state: SET_STATES[i] as u16,
                flags: SET_FLAGS,
            },
            list,
        );
    }
    put(
        ListKey {
            state: RUNEWORD_STATE as u16,
            flags: RUNEWORD_FLAGS,
        },
        &it.runeword_list,
    );
}
