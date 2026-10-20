//! The item queries and socket effects that the inventory seams answer
//! from d2-sim's own state: the item tables, the item store, the stat
//! lists, the unit records and the inventory state. The [`InvRest`]
//! methods of the same names are no longer asked by [`InvDesk`].
//! Nothing here decides a rule: the rules are `items::inventory`'s and
//! `items::props`'s; this file gathers their inputs.

use super::{InvDesk, InvError, InvRest};
use crate::items::affixes::affix;
use crate::items::inventory::{level_requirement, AffixReq, LevelReqItem, LevelReqUnit};
use crate::items::recharge::{self, Recharged};
use crate::items::{props, replenish_timer_at, stat};
use crate::stats::{key_layer, key_stat};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::record::flags2;
use crate::units::{UnitId, UnitType};

/// Stat 92 `item_levelreq`.
pub const STAT_LEVELREQ: u16 = 92;
/// Stat 97 `item_nonclassskill`.
pub const STAT_NONCLASSSKILL: u16 = 97;
/// Stat 107 `item_singleskill`.
pub const STAT_SINGLESKILL: u16 = 107;
/// Entries the extended-list read copies per stat (at most 64).
const MAX_ENTRIES: usize = 64;
/// Item timer event 3: replenish.
const EVENT_REPLENISH: u32 = 3;

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// The items class of an item unit (its item data's record).
    pub(super) fn class_of(&self, u: UnitId) -> Option<usize> {
        self.econ.items.get(u).map(|i| i.record)
    }

    /// Two-handed: items `2handed` ≠ 0.
    pub fn is_two_handed(&self, u: UnitId) -> bool {
        self.class_of(u)
            .and_then(|r| self.tables.item(r))
            .is_some_and(|r| r.twohanded != 0)
    }

    /// Ammo type: the primary type's itemtypes `shoots`
    /// (a bow shoots arrows), as an itemtypes row (−1: the link's miss value, which
    /// no equivalence test matches). `None`: no item record or type row, or
    /// `shoots` 0 (blank: row 0 is `None`, and only launchers have a
    /// non-zero link), so the ammo pairing never pairs two
    /// non-launchers.
    pub fn ammo_of(&self, u: UnitId) -> Option<i16> {
        let r = self.class_of(u)?;
        let shoots = self.econ.tables.itype_of(r)?.shoots;
        (shoots != 0).then_some(shoots as i16)
    }

    /// Book / scroll spell: the item's spell index (suffix slot
    /// 0). 0 without item data.
    pub fn spell_of(&self, u: UnitId) -> i32 {
        self.econ.items.get(u).map_or(0, |i| i32::from(i.suffix[0]))
    }

    /// The unit the level requirement is computed for: class, whether it is a
    /// player, unit flag 0x2000000.
    pub fn level_req_unit(&self, unit: UnitId) -> Option<LevelReqUnit> {
        let r = self.econ.units.get(unit)?;
        Some(LevelReqUnit {
            class: r.class,
            player: r.ty == UnitType::Player,
            expansion: r.flags2 & flags2::EXPANSION != 0,
        })
    }

    /// The entries of `stat` in the item's extended stat list
    /// (at most 64, list order) as (layer, value); a plain
    /// list or none gives none.
    pub(super) fn extended_entries(&self, u: UnitId, s: u16) -> Vec<(u16, i32)> {
        let stats = &*self.econ.stats;
        let Some(l) = stats.unit_list(u).filter(|&l| stats.is_extended(l)) else {
            return Vec::new();
        };
        stats
            .full_entries(l)
            .into_iter()
            .filter(|&(k, _)| key_stat(k) == s)
            .map(|(k, v)| (key_layer(k), v))
            .take(MAX_ENTRIES)
            .collect()
    }

    /// Everything the level requirement reads of `u`: the item's affix rows
    /// (prefixes and suffixes; id 0 or unknown: none), the
    /// set / unique `lvl req` of the file index, the version, items
    /// `levelreq`, the items of the item's own inventory (recursive), the
    /// skills of stats 107 and 97 (skill ids ≥ the skills count skipped)
    /// and stat 92. No item data → quality 2 and nothing else.
    pub fn level_req_item(&self, u: UnitId) -> LevelReqItem {
        let Some(it) = self.econ.items.get(u) else {
            return LevelReqItem {
                quality: 2,
                ..LevelReqItem::default()
            };
        };
        let t = self.econ.tables;
        let aff = |id: u16| {
            affix(t, id).map(|a| AffixReq {
                levelreq: a.levelreq,
                class: a.class,
                classlevelreq: a.classlevelreq,
            })
        };
        let row = usize::try_from(it.file_index).ok();
        let skill = |layer: u16| t.skills.get(usize::from(layer));
        LevelReqItem {
            quality: it.quality,
            automagic: aff(it.auto_affix),
            prefixes: it.prefix.map(aff),
            suffixes: it.suffix.map(aff),
            set_lvlreq: row.and_then(|i| t.setitems.get(i)).map_or(0, |s| s.lvl_req),
            unique_lvlreq: row.and_then(|i| t.uniques.get(i)).map(|r| r.lvl_req),
            version: it.format,
            class_levelreq: self
                .tables
                .item(it.record)
                .map_or(0, |r| i32::from(r.levelreq)),
            fillers: self
                .state
                .inventories
                .get(&u)
                .map(|inv| {
                    inv.items()
                        .iter()
                        .map(|&f| self.level_req_item(f))
                        .collect()
                })
                .unwrap_or_default(),
            single_skills: self
                .extended_entries(u, STAT_SINGLESKILL)
                .into_iter()
                .filter_map(|(l, _)| skill(l).map(|s| s.reqlevel))
                .collect(),
            nonclass_skills: self
                .extended_entries(u, STAT_NONCLASSSKILL)
                .into_iter()
                .filter_map(|(l, _)| skill(l).map(|s| (s.reqlevel, i32::from(s.charclass))))
                .collect(),
            stat_levelreq: self.econ.stats.unit_total(u, STAT_LEVELREQ, 0),
        }
    }

    /// The item level requirement of `item` for `unit` (none: no unit given, or
    /// no unit record).
    pub fn item_level_requirement(&self, item: UnitId, unit: Option<UnitId>) -> i32 {
        let who = unit.and_then(|u| self.level_req_unit(u));
        level_requirement(&self.level_req_item(item), who.as_ref())
    }

    /// Socket filler properties on a filler just linked into `target`: the
    /// filler's gems row and the target's apply type (items
    /// `gemapplytype`: weapon, armor/helm or shield); the properties land in the filler's own list.
    pub fn apply_filler_properties(&mut self, filler: UnitId, target: UnitId) {
        let apply = self
            .class_of(target)
            .and_then(|r| self.econ.tables.item(r))
            .map_or(0, |r| r.gemapplytype);
        // A rune's mode 5 passes the socketed item as the extra unit.
        if let Err(e) = self.econ.with_item(filler, |s| {
            let mut on = s.unit_stats(target);
            props::apply_socket_filler_into(s.tables, s.item, apply, Some(&mut on))
        }) {
            self.state.errors.push(InvError::Economy(e));
        }
        // The filler's root list (not its property list) is
        // attached under the target, so the filler keeps its own properties
        // and the target's totals include them.
        if let Some(l) = self.econ.stats.unit_list(filler) {
            self.econ
                .stats
                .equip(&mut *self.econ.hooks, target, Some(l), false, true);
        }
        self.sync_in();
    }

    /// The runeword step of a socket fill on `target`:
    /// the runeword record that matches the target's inventory, then
    /// its properties (a `server` row needs the expansion flag; a target that
    /// already has a runeword list is left as is), then the replenish
    /// timers (event 3 when not scheduled).
    /// True when the runeword properties ran. The `server` gate decides
    /// the expansion question.
    pub fn activate_runeword_on(&mut self, target: UnitId) -> bool {
        self.socket_runeword(target).0
    }

    /// The socket count (stat 194) and the filler class ids of `target`'s
    /// inventory in insertion order: the inputs of the runeword match.
    fn runeword_inputs(&self, target: UnitId) -> (u8, Vec<usize>) {
        let fillers = self
            .state
            .inventories
            .get(&target)
            .map(|inv| {
                inv.items()
                    .iter()
                    .filter_map(|&f| self.class_of(f))
                    .collect()
            })
            .unwrap_or_default();
        let sockets = self.econ.stats.unit_total(target, stat::NUMSOCKETS, 0) as u8;
        (sockets, fillers)
    }

    /// Whether `target` still matches a runeword row.
    pub fn runeword_matches(&self, target: UnitId) -> bool {
        let Some(it) = self.econ.items.get(target) else {
            return false;
        };
        let (sockets, fillers) = self.runeword_inputs(target);
        props::runeword_row(self.econ.tables, it.record, it.quality, sockets, &fillers).is_some()
    }

    /// [`InvDesk::activate_runeword_on`], then the recharge
    /// that the socket fill runs after the timers
    /// whenever a runeword row matched. Returns (runeword ran, the
    /// recharged charged skills for the S→C 0x3E that follows).
    pub fn socket_runeword(&mut self, target: UnitId) -> (bool, Vec<Recharged>) {
        let Some(it) = self.econ.items.get(target) else {
            return (false, Vec::new());
        };
        let (sockets, fillers) = self.runeword_inputs(target);
        let Some(row) =
            props::runeword_row(self.econ.tables, it.record, it.quality, sockets, &fillers)
        else {
            return (false, Vec::new());
        };
        let ladder = self.econ.fields.ladder;
        let scheduled = self
            .econ
            .game
            .timers
            .unit_timers(target)
            .into_iter()
            .any(|id| {
                self.econ
                    .game
                    .timers
                    .event(id)
                    .is_some_and(|(e, _, _)| u32::from(e) == EVENT_REPLENISH)
            });
        let frame = self.econ.game.frame as u32;
        let rate = self.econ.game.tick_rate;
        // Runeword properties: I is the filler just inserted (the last
        // of the socketed item's inventory list), O the socketed item.
        let Some(filler) = self
            .state
            .inventories
            .get(&target)
            .and_then(|inv| inv.items().last().copied())
        else {
            return (false, Vec::new());
        };
        let ran = self.econ.with_item(filler, |s| {
            let mut socketed = s.unit_stats(target);
            props::activate_runeword(s.tables, s.item, &mut socketed, row, ladder)
        });
        let ran = ran.and_then(|ran| {
            if !ran {
                return Ok((false, None));
            }
            if let Some(i) = self.econ.items.get_mut(target) {
                i.flags |= crate::items::flag::RUNEWORD;
            }
            self.econ.with_item(target, |s| {
                (
                    true,
                    replenish_timer_at(&s.item.stats, scheduled, frame, rate),
                )
            })
        });
        self.sync_in();
        let ran = match ran {
            Ok((ran, at)) => {
                if let Some(f) = at {
                    // TODO: the event's arguments are
                    // not written; scheduled with 0, 0 (as at creation).
                    if let Err(e) =
                        self.econ
                            .game
                            .schedule_event(target, EVENT_REPLENISH, f as i32, None, 0, 0)
                    {
                        self.state.errors.push(InvError::Economy(e.into()));
                    }
                }
                ran
            }
            Err(e) => {
                self.state.errors.push(InvError::Economy(e));
                false
            }
        };
        (ran, self.recharge_item(target))
    }

    /// Recharge on `item`: its
    /// stat-204 entries of the extended list; each below its maximum is
    /// set on the item's own lists, else on the items of
    /// its own inventory in node order until one takes it.
    pub fn recharge_item(&mut self, item: UnitId) -> Vec<Recharged> {
        let entries = self.extended_entries(item, recharge::CHARGED_SKILL);
        let inner: Vec<UnitId> = self
            .state
            .inventories
            .get(&item)
            .map(|inv| inv.items().to_vec())
            .unwrap_or_default();
        let mut sets = Vec::new();
        let out = recharge::recharge(&entries, |k, m| sets.push((k, m)));
        for (k, m) in sets {
            for u in std::iter::once(item).chain(inner.iter().copied()) {
                match self
                    .econ
                    .with_item(u, |s| recharge::set_charges(&mut s.item.stats, k, m))
                {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(e) => self.state.errors.push(InvError::Economy(e)),
                }
            }
        }
        self.sync_in();
        out
    }

    /// Repair of a broken item, steps
    /// 1 and 2's list entry: item flag 0x200 set and 0x100 cleared, the item
    /// appended to its owner's update list (the deferred S→C 0x7D
    /// follows in the update pass). The
    /// durability (step 3) is the caller's.
    ///
    /// TODO: the stat link and refresh,
    /// the set-item update and the weapon bookkeeping are not run here.
    pub fn repair_broken_item(&mut self, item: UnitId) {
        let Some(d) = self.state.items.get_mut(&item) else {
            return;
        };
        d.flags =
            (d.flags | crate::items::moves::iflag::REPAIRED) & !crate::items::moves::iflag::BROKEN;
        let owner = d.inv;
        let guid = self.guid_of(item);
        if let Some(o) = owner.and_then(|u| self.owner_of(u)) {
            self.with_inv(o, |inv, _| inv.push_update(guid));
        }
        // Owner refresh (step 2): the update pass runs.
        if let Some(u) = owner {
            crate::items::inventory::InvWorld::owner_refresh(self, u);
        }
        self.sync_out();
        self.flush_equip();
    }

    /// The break on the inventory model (a weapon or armor whose durability
    /// hit 0; the item's stat 72 := 0 and its 0x3E are the action wiring's):
    /// flag 0x100, the owner's update list entry, the owner refresh.
    pub fn break_item(&mut self, item: UnitId) {
        let Some(d) = self.state.items.get_mut(&item) else {
            return;
        };
        d.flags |= crate::items::moves::iflag::BROKEN;
        let owner = d.inv;
        let equipped = d.mode == 1;
        let guid = self.guid_of(item);
        // The weapon-in-use unlink and the deactivation
        // of an item in mode 1: its stat list leaves the owner.
        if let (Some(u), true) = (owner, equipped) {
            self.weapon_link_on(u, item, false);
            self.unlink_item_stats(u, item);
        }
        if let Some(o) = owner.and_then(|u| self.owner_of(u)) {
            self.with_inv(o, |inv, _| inv.push_update(guid));
        }
        if let Some(u) = owner {
            crate::items::inventory::InvWorld::owner_refresh(self, u);
        }
        self.sync_out();
        self.flush_equip();
    }

    /// Replenish timers of `u` (self-repair and self-recharging ammo
    /// items): event 3 at frame + 2500 / r + 1 for the
    /// first of stats 252, 253 with a total r ≠ 0, when none is scheduled.
    pub fn schedule_replenish(&mut self, u: UnitId) {
        let scheduled = self.econ.game.timers.unit_timers(u).into_iter().any(|id| {
            self.econ
                .game
                .timers
                .event(id)
                .is_some_and(|(e, _, _)| u32::from(e) == EVENT_REPLENISH)
        });
        let frame = self.econ.game.frame as u32;
        let rate = self.econ.game.tick_rate;
        let at = match self.econ.with_item(u, |s| {
            replenish_timer_at(&s.item.stats, scheduled, frame, rate)
        }) {
            Ok(at) => at,
            Err(e) => {
                self.state.errors.push(InvError::Economy(e));
                None
            }
        };
        if let Some(f) = at {
            if let Err(e) = self
                .econ
                .game
                .schedule_event(u, EVENT_REPLENISH, f as i32, None, 0, 0)
            {
                self.state.errors.push(InvError::Economy(e.into()));
            }
        }
    }
}
