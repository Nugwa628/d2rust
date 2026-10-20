//! The player's items through a save: the item list written from the
//! wired host's economy and inventory model ([`WiredWorld::save_items`],
//! `adapters::character::save::item_list` over an inventory desk) and the
//! list read back onto a new player unit ([`WiredWorld::load_items`],
//! `InvDesk::load_entry`). Part of the wired world host: the character
//! save adapter writes a save through [`WiredWorld::save_items`] and the
//! join reads it back through [`WiredWorld::load_items`].

use d2_formats::d2s::ItemEntry;
use d2_sim::items::bitstream::read::read_save_entry;
use d2_sim::items::inventory::UnitKind;
use d2_sim::items::moves::MoveUnits;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::inventory::load::LoadFault;

use super::{inv_take_sent, ActionEvents, Game, WiredWorld};
use crate::adapters::handlers::items::moves::preview_skills::{sync_oskills, SKILL_SHRINE_STATE};
use d2_formats::d2s::appearance::Equipment;

use crate::adapters::character::save::{equipment, item_list, SaveContext};

/// Stats 6 and 8: hitpoints and mana.
const HITPOINTS: u16 = 6;
const MANA: u16 = 8;

/// What the load of a save's item list did ([`WiredWorld::load_items`]).
#[derive(Debug, Default)]
pub struct LoadedItems {
    /// Each placed top-level item, in list order.
    pub items: Vec<UnitId>,
    /// What the placements queued (receiving unit, bytes), in send order:
    /// the join's item messages.
    pub sent: Vec<(UnitId, Vec<u8>)>,
    /// Why an entry made no item (record unreadable, no room, no model).
    pub faults: Vec<String>,
}

impl<R, S> WiredWorld<R, S> {
    /// Frees a leaving player's belongings: every item it holds (body,
    /// belt, inventory, cube and stash pages, the cursor; socketed fillers
    /// first), then its mercenary's items and the mercenary unit, and the
    /// player's hireling list. The player unit is the caller's to remove.
    /// Returns one text per fault.
    pub fn free_player_belongings<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
    ) -> Vec<String> {
        let mut faults = Vec::new();
        let merc = self.hireling_unit(game, player);
        let owners: Vec<UnitId> = std::iter::once(player).chain(merc).collect();
        match self.inventory.take() {
            Some(mut inv) => {
                self.with_economy(game, events, |econ, _| {
                    let mut d = inv.desk(econ);
                    for &owner in &owners {
                        let mut items = d.state.items_of(owner);
                        items.extend(d.state.cursor_of(owner));
                        for item in items {
                            d.remove(owner, item);
                            free_item_tree(&mut d, item);
                        }
                    }
                });
                for owner in &owners {
                    inv.state.inventories.remove(owner);
                }
                self.inventory = Some(inv);
            }
            None => faults.push("no inventory model: the player's items stay".into()),
        }
        if let Some(m) = merc {
            events.lend_world(|a| {
                a.with(game, |g, v| {
                    v.leave_room(g, m, true);
                    v.remove(g, m);
                })
            });
        }
        self.state.hirelings.lists.remove(&player);
        faults
    }

    /// The save entries of `player`'s items.
    /// `Err`: no inventory model, or an item that does not fit the buffer.
    pub fn save_items<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
    ) -> Result<Vec<ItemEntry>, String> {
        let Some(mut inv) = self.inventory.take() else {
            return Err("no inventory model (WiredWorld::inventory is None)".into());
        };
        let out = self.with_economy(game, events, |econ, _| {
            let isc = econ.tables.isc.clone();
            let mut d = inv.desk(econ);
            let out = item_list(&d, player, &isc).map_err(|e| e.to_string());
            // The writer's changes stay on the items (every later save and message sees them).
            d.apply_write_backs();
            out
        });
        self.inventory = Some(inv);
        out
    }

    /// The appearance inputs of `player`'s items
    /// (`adapters::character::save::equipment` over an inventory desk) with `ctx`'s weapon class and states. `Err`: no inventory
    /// model.
    pub fn save_equipment<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        ctx: &SaveContext,
    ) -> Result<Equipment, String> {
        let Some(mut inv) = self.inventory.take() else {
            return Err("no inventory model (WiredWorld::inventory is None)".into());
        };
        let out = self.with_economy(game, events, |econ, _| {
            let d = inv.desk(econ);
            equipment(&d, player, ctx)
        });
        self.inventory = Some(inv);
        Ok(out)
    }

    /// Makes the items of a save's list on `player` and places them
    /// as a save's item list is read. The player's inventory is added to
    /// the model when it has none, as [`WiredWorld::start_items`] does.
    pub fn load_items<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        entries: &[ItemEntry],
    ) -> LoadedItems {
        self.load_list(game, events, player, entries, false)
    }

    /// The player data's mouse-skill slots hold the decoded left and right mouse
    /// skills (skill, item index, −1 none) until the post-load resolves
    /// them. The inventory rest's saved slots take them before the items
    /// load, so the weapon bookkeeping's restore reads them when an
    /// equipped item makes the current skill unusable.
    pub fn seed_saved_mouse_skills<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        left: (i32, i32),
        right: (i32, i32),
    ) {
        let Some(mut inv) = self.inventory.take() else {
            return;
        };
        self.with_economy(game, events, |econ, _| {
            if let Some(guid) = econ.units.get(player).map(|u| u.guid) {
                let o = d2_sim::items::moves::Owner::player(guid);
                inv.rest.set_saved_mouse_skill(o, true, left);
                inv.rest.set_saved_mouse_skill(o, false, right);
            }
        });
        self.inventory = Some(inv);
    }

    /// The player's saved mouse-skill slots (left and
    /// right, each (skill, owner GUID, −1 = class skill))
    /// as the item load's weapon bookkeeping left them; `None` without
    /// the inventory model.
    pub fn saved_mouse_skills<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
    ) -> Option<[(i32, i32); 2]> {
        let inv = self.inventory.take()?;
        let mut out = None;
        self.with_economy(game, events, |econ, _| {
            if let Some(guid) = econ.units.get(player).map(|u| u.guid) {
                let o = d2_sim::items::moves::Owner::player(guid);
                out = Some([
                    inv.rest.saved_mouse_skill(o, true),
                    inv.rest.saved_mouse_skill(o, false),
                ]);
            }
        });
        self.inventory = Some(inv);
        out
    }

    /// [`Self::load_items`] for a corpse unit's list: placed by the
    /// corpse placement (`InvDesk::load_corpse_entry`).
    pub fn load_corpse_items<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        corpse: UnitId,
        entries: &[ItemEntry],
    ) -> LoadedItems {
        self.load_list(game, events, corpse, entries, true)
    }

    /// The `kf` reader makes the golem item as a unit, placed nowhere;
    /// the join's re-cast equips it ([`Self::place_golem_item`]). Returns the
    /// fault text when no unit was made.
    pub fn make_golem_item<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        entry: &ItemEntry,
    ) -> Result<(), String> {
        let Some(mut inv) = self.inventory.take() else {
            return Err("no inventory model (WiredWorld::inventory is None)".into());
        };
        let mut out: Result<(UnitId, ItemEntry), String> = Err("no item unit".to_string());
        self.with_economy(game, events, |econ, _| {
            let mut d = inv.desk(econ);
            out = match read_save_entry(&entry.bytes, d.econ.tables) {
                Ok(rec) => d
                    .make_entry(&rec)
                    .map(|u| (u, entry.clone()))
                    .map_err(|f| fault(&f).to_string()),
                Err(e) => Err(format!("unreadable record: {e:?}")),
            };
            d.sync_out();
            // The kf reader sets the item to mode 3, whose re-init gives
            // the frame count 0x1100; the later equip does not change it.
            if let Ok((u, _)) = &out {
                if let Some(r) = d.econ.units.get_mut(*u) {
                    r.anim.frame_count = 0x1100;
                }
            }
        });
        self.inventory = Some(inv);
        out.map(|made| {
            self.golem_loose.insert(player, made);
        })
    }

    /// The end of the join's re-cast: the loose golem item equipped on the
    /// new golem as saved. A player
    /// without a loose item, or without a golem, does nothing.
    pub fn place_golem_item<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
    ) -> Vec<String> {
        let mut faults = Vec::new();
        let mut placed: Option<u8> = None;
        let Some((unit, _entry)) = self.golem_loose.remove(&player) else {
            return faults;
        };
        let Some(golem) = self.golem_unit(game, events, player) else {
            return faults;
        };
        let Some(mut inv) = self.inventory.take() else {
            return faults;
        };
        self.with_economy(game, events, |econ, _| {
            let Some((class, guid)) = econ.units.get(golem).map(|u| (u.class, u.guid)) else {
                return;
            };
            if !inv.state.inventories.contains_key(&golem) {
                inv.state
                    .add_inventory(golem, UnitKind::Monster { class }, guid);
            }
            super::wired::lend_skills(econ, &mut inv);
            let mut d = inv.desk(econ);
            if d.equip_first_loc(golem, unit) {
                // The new monster's add messages carry its equipment
                // (0x9D action 6, `monster_add.rs`), which clears the item's
                // changed flag; nothing is left for the owner's update pass
                // (the stream is 0x9D after the add's mode message, no 0x48).
                let glob = d2_sim::items::moves::Owner {
                    ty: d2_sim::items::moves::Owner::MONSTER,
                    guid,
                };
                d.update_done(glob);
                if let Some(loc) = d.state.items.get(&unit).map(|i| i.body_loc) {
                    placed = Some(loc);
                }
            } else {
                faults.push("the golem item did not take".into());
            }
            d.sync_out();
            let _ = inv_take_sent(&mut d);
            super::wired::return_skills(econ, &mut inv, false);
        });
        self.inventory = Some(inv);
        if let Some(loc) = placed {
            events
                .action()
                .hooks()
                .monster_equip
                .entry(golem)
                .or_default()
                .insert(loc, unit);
        }
        faults
    }

    /// Adds `player`'s inventory to the model when it has none.
    fn ensure_player_inventory<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
    ) {
        let Some(mut inv) = self.inventory.take() else {
            return;
        };
        self.with_economy(game, events, |econ, _| {
            let Some((class, guid)) = econ.units.get(player).map(|u| (u.class, u.guid)) else {
                return;
            };
            if !inv.state.inventories.contains_key(&player) {
                inv.state
                    .add_inventory(player, UnitKind::Player { class: class as u8 }, guid);
            }
        });
        self.inventory = Some(inv);
    }

    fn load_list<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        entries: &[ItemEntry],
        corpse: bool,
    ) -> LoadedItems {
        let mut r = LoadedItems::default();
        if entries.is_empty() {
            // A player's inventory exists from the unit's allocation, items
            // or not: without it a pick-up finds no inventory.
            if !corpse {
                self.ensure_player_inventory(game, events, player);
            }
            return r;
        }
        let Some(mut inv) = self.inventory.take() else {
            r.faults
                .push("no inventory model (WiredWorld::inventory is None)".into());
            return r;
        };
        self.with_economy(game, events, |econ, _| {
            let Some((ty, class, guid)) = econ.units.get(player).map(|u| (u.ty, u.class, u.guid))
            else {
                r.faults.push(format!("no player unit {player:?}"));
                return;
            };
            if !inv.state.inventories.contains_key(&player) {
                // The hireling and the golem lists are monsters'.
                let kind = if ty == UnitType::Monster {
                    UnitKind::Monster { class }
                } else {
                    UnitKind::Player { class: class as u8 }
                };
                inv.state.add_inventory(player, kind, guid);
            }
            // The skill lists lent to the rest: the item-skill link's 0x22
            // and the stat 97 / 107 callback's 0x21 of the join's item
            // calls, in item order.
            if !corpse {
                super::wired::lend_skills(econ, &mut inv);
            }
            let mut d = inv.desk(econ);
            // The loader remembers hitpoints and mana before the items and
            // restores them after (item bonuses do not change them: the
            // max-rescale must not move the stored values).
            let remembered = [HITPOINTS, MANA].map(|s| d.econ.stats.unit_total(player, s, 0));
            for (i, e) in entries.iter().enumerate() {
                let rec = match read_save_entry(&e.bytes, d.econ.tables) {
                    Ok(rec) => rec,
                    Err(err) => {
                        r.faults
                            .push(format!("item {i}: unreadable record: {err:?}"));
                        continue;
                    }
                };
                let made = if corpse {
                    d.load_corpse_entry(player, &rec)
                } else {
                    d.load_entry(player, &rec)
                };
                match made {
                    Ok(u) => r.items.push(u),
                    Err(f) => r.faults.push(format!("item {i}: {}", fault(&f))),
                }
                // The stat callback of this item's attach, so its 0x21
                // follows the item's own messages.
                if !corpse {
                    if let Some(mut st) = d.rest.take_skills() {
                        let mut sent = Vec::new();
                        let o = d2_sim::items::moves::Owner::player(guid);
                        let changes = std::mem::take(&mut d.econ.hooks.oskill_changes);
                        if let Some(unit) = st.lists.get(&o).map(|sl| sl.unit) {
                            let mine: Vec<_> = changes
                                .iter()
                                .filter(|c| c.0 == unit)
                                .map(|c| (c.1, c.2, c.3))
                                .collect();
                            sync_oskills(
                                &mut st,
                                o,
                                &mine,
                                |sl, stat, skill| {
                                    d.econ.stats.unit_total(sl.unit, stat, skill as u16)
                                },
                                |sl| d.econ.stats.has_state(sl.unit, SKILL_SHRINE_STATE),
                                &mut sent,
                            );
                        }
                        d.rest.queue_sent(sent);
                        d.rest.stage_skills(st, d.tables);
                    }
                }
            }
            // The placements put the items on the player's update list
            // The join sends that list as the player's item messages
            // (0x9C / 0x9D) and resets it, so the first tick's unit update
            // does not send them after the join sequence. Run here at the
            // list's end, as the start items do: the load's later steps
            // (corpse, hireling, quest entry, mouse skills) never touch the player's
            // list. D2RUST's own placement of the two calls, equal in effect:
            // the join streams of the combat-potion-midfight and the cube
            // small-rejuvs checks (frame 2: 0x9D, 0x9C × 2 before the 0x23
            // pair) are pinned byte for byte.
            let owner = d2_sim::items::moves::Owner::player(guid);
            let mut listed = Vec::new();
            if !corpse && ty == UnitType::Player && d.update_bits(owner) & 1 != 0 {
                match d2_sim::items::moves::update_list_pass(&mut d, guid, guid) {
                    Ok(m) => listed = m,
                    Err(e) => r.faults.push(format!("update list: {e:?}")),
                }
                d.update_done(owner);
            }
            d.sync_out();
            if !corpse {
                for (s, v) in [HITPOINTS, MANA].into_iter().zip(remembered) {
                    d.econ.stats.unit_set(&mut *d.econ.hooks, player, s, v, 0);
                }
            }
            r.sent = inv_take_sent(&mut d)
                .into_iter()
                .filter_map(|(u, b)| Some((u?, b)))
                .collect();
            r.sent.extend(listed.into_iter().map(|m| (player, m)));
            if !corpse {
                super::wired::return_skills(econ, &mut inv, false);
            }
        });
        self.inventory = Some(inv);
        r
    }
}

fn fault(f: &LoadFault) -> &'static str {
    match f {
        LoadFault::NotCreated => "no item unit could be made from the record",
        LoadFault::NoInventory => "the owner has no inventory",
        LoadFault::NoRoom => "no place for the item (freed)",
        LoadFault::StaleRuneword => "runeword flag without a matching runeword (removed)",
        LoadFault::RecordFailed => "the record failed to read (entry skipped)",
    }
}

/// Frees `item` and, first, every item socketed into it.
fn free_item_tree<H, R>(d: &mut d2_sim::wiring::inventory::InvDesk<'_, '_, H, R>, item: UnitId)
where
    H: d2_sim::units::lifecycle::LifecycleHooks,
    R: d2_sim::wiring::inventory::InvRest + ?Sized,
{
    for filler in d.state.fillers(item) {
        free_item_tree(d, filler);
    }
    d.state.inventories.remove(&item);
    d.free(item);
}
