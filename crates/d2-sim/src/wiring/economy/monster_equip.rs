//! Monster equipment: the item made from its code (the monster as its
//! source, spawn mode 4, no sockets, never ethereal, item level L, no
//! seeds), marked as monster equipment, put on its body location
//! (equipped with the stat link to the wearer) and its durability filled.
//! Used by monster init, summon equipment and the monster free.
//!
//! The monster's inventory on this wiring is the holdings
//! [`ActionHooks::monster_equip`]; the item's place is its static path
//! (x = body location, y = 0, as for every inventory item). The player
//! equip checks and the item-level flag changes beyond unit flag 0x2,
//! mode 1 and the stat link are not run: a monster's equipment comes
//! from its own tables and is never checked against what it can wear.

use super::quest_reward::max_durability;
use super::{DeathDrops, Economy, GameFields, ItemSpawn, UnitStats};
use crate::items::{flag, stat, CreateError, ItemGame, ItemRequest, ItemStats};
use crate::path::{StaticPath, UnitPath};
use crate::treasure::runtime::ItemData;
use crate::units::hooks::Sim;
use crate::units::UnitId;
use crate::wiring::action::{ActionHooks, Pending};

/// Unit mode of an item created into an inventory (spawn mode 4).
const SPAWN_INVENTORY: u32 = 4;
/// Item mode 1: equipped.
const MODE_EQUIPPED: u32 = 1;

/// Creates `code` for `unit` at body location `loc` with `quality` and
/// item level `ilvl`; the item, or none (unknown code, the creation was
/// refused). Draws on the game seed and the item's own seeds
/// ([`Economy::create_item`]).
#[allow(clippy::too_many_arguments)]
pub fn create_monster_equip<X: Pending>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    unit: UnitId,
    code: [u8; 4],
    loc: u8,
    quality: u8,
    ilvl: i32,
) -> Option<UnitId> {
    let t = d.tables.clone();
    // A drop state without pick rows looks the code up in its item tables
    // (the same combined array, as `unit_quest_drop`).
    let index = if d.picks.rows.is_empty() {
        crate::treasure::class_pick::ClassPicks::of_items(&t.items.items, t.items.parts)
            .find_code(code)?
    } else {
        d.picks.find_code(code)?
    };
    let mut fields = GameFields::from_action(
        h.game_seed,
        &h.ai_info,
        sim.data.expansion,
        std::mem::take(&mut h.uniques),
    );
    let mut items = std::mem::take(&mut h.items);
    let made = {
        let mut econ = Economy {
            game: &mut *sim.game,
            units: &mut *sim.units,
            stats: &mut *sim.stats,
            data: sim.data,
            hooks: &mut *h,
            fields: &mut fields,
            tables: &t.items,
            items: &mut items,
        };
        let mut rq = ItemRequest {
            unit: econ.request_unit(unit, None),
            ilvl: ilvl.max(1),
            item: index as i32,
            format: ItemGame::item_format(&*econ.fields),
            quality,
            flags2: crate::items::req::NEVER_ETHEREAL | crate::items::req::NO_SOCKETS,
            ..ItemRequest::default()
        };
        let spawn = ItemSpawn {
            room: None,
            mode: SPAWN_INVENTORY,
            init_flags: 1,
        };
        match econ.create_item(&mut rq, false, spawn) {
            Ok(item) => {
                if let Some(i) = econ.items.get_mut(item) {
                    // Identified on success; flag 0x1000 (no sell).
                    i.flags |= flag::NOSELL | flag::IDENTIFIED;
                }
                Some(item)
            }
            Err(e) => {
                if !matches!(
                    e,
                    crate::wiring::economy::EconomyError::Create(CreateError::BadIndex)
                ) {
                    d.failures.push(e);
                }
                None
            }
        }
    };
    h.items = items;
    h.game_seed = fields.seed;
    h.uniques = std::mem::take(&mut fields.uniques);
    d.fields = fields;
    let item = made?;
    if loc != 0 {
        equip(h, sim, &t.items, unit, item, loc);
    }
    Some(item)
}

/// Equips `item` on a monster at `loc` (as far as the wiring holds the
/// equip steps), then fills the durability.
fn equip<X: Pending>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    tables: &crate::items::ItemTables,
    unit: UnitId,
    item: UnitId,
    loc: u8,
) {
    put_on(h, sim, unit, item, loc);
    // The durability fill.
    let mut fields = GameFields::from_action(
        h.game_seed,
        &h.ai_info,
        sim.data.expansion,
        std::mem::take(&mut h.uniques),
    );
    let mut items = std::mem::take(&mut h.items);
    {
        let mut econ = Economy {
            game: &mut *sim.game,
            units: &mut *sim.units,
            stats: &mut *sim.stats,
            data: sim.data,
            hooks: &mut *h,
            fields: &mut fields,
            tables,
            items: &mut items,
        };
        let max = max_durability(&econ, item);
        if max > 0 {
            econ.with_stats(|ctx| UnitStats::new(ctx, item).set_base(stat::DURABILITY, 0, max));
        }
    }
    h.items = items;
    h.uniques = std::mem::take(&mut fields.uniques);
}

/// Puts `item` on a monster, as far as the wiring holds the equip steps:
/// unit flag 0x2 cleared (no longer a skill target), mode 1, the place (the
/// static path, x = body location), page 0xFF, the stat link to the wearer
/// and the holdings entry. Also the Iron Golem's take of its item.
pub fn put_on<X: Pending>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
    item: UnitId,
    loc: u8,
) {
    if let Some(r) = sim.units.get_mut(item) {
        r.flags &= !crate::units::record::flags::TILE;
        r.mode = MODE_EQUIPPED;
    }
    // The item's place as its static path (x = body location).
    if let Some(paths) = h.paths.as_mut() {
        let mut p = StaticPath::default();
        p.set(None, i32::from(loc), 0);
        paths.records.insert(item, UnitPath::Static(p));
    }
    if let Some(i) = h.items.get_mut(item) {
        i.inv_page = 0xFF;
    }
    if let Some(l) = sim.stats.unit_list(item) {
        sim.stats.equip(&mut *h, unit, Some(l), false, true);
    }
    h.monster_equip.entry(unit).or_default().insert(loc, item);
}

/// The inventory part of freeing a monster: unless `class` is a monstats row
/// with `interact`, every item the monster holds is freed (taken off and out
/// of the inventory, then removed, in inventory-chain order = item GUID
/// order) and the holdings entry goes. An `interact` monster keeps its
/// inventory.
pub fn free_monster_inventory<X: Pending>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    unit: UnitId,
    class: u32,
) {
    let interact = h
        .tables
        .combat
        .monstats
        .get(class as usize)
        .is_some_and(|m| m.interact);
    if interact {
        return;
    }
    let Some(held) = h.monster_equip.remove(&unit) else {
        return;
    };
    let mut items: Vec<UnitId> = held.into_values().collect();
    items.sort_by_key(|&i| sim.units.get(i).map_or(u32::MAX, |r| r.guid));
    for item in items {
        if sim.units.get(item).is_none() {
            continue;
        }
        h.items.remove(item);
        if let Err(e) = crate::units::lifecycle::remove(sim, &mut *h, item) {
            h.errors.push(crate::wiring::action::WiringError::Unit(e));
        }
    }
}

/// The code an ancient barbarian's equipment entry makes on `difficulty`:
/// the items row of `code` (the first row with that code) gives `ubercode` on 1 and `ultracode` on 2;
/// difficulty 0, or a code with no row, keeps `code`.
pub fn tier_code(items: &[ItemData], code: [u8; 4], difficulty: u8) -> [u8; 4] {
    match (items.iter().find(|r| r.code == code), difficulty) {
        (Some(r), 1) => r.ubercode,
        (Some(r), 2) => r.ultracode,
        _ => code,
    }
}

/// The item `unit` holds at body location `loc` (and still exists).
pub fn item_at<X>(h: &ActionHooks<X>, sim: &Sim<'_>, unit: UnitId, loc: u8) -> Option<UnitId> {
    h.monster_equip
        .get(&unit)?
        .get(&loc)
        .copied()
        .filter(|&i| sim.units.get(i).is_some())
}

/// Summon equipment for monster `m` of `owner`:
/// [`crate::monsters::init::monequip_rows`]
/// on `m`'s own seed with the host's [`create_monster_equip`] and the
/// monequip rows of [`crate::wiring::action::ActionTables::monequip`].
/// A row of four spaces copies the code of the owner's item at that body
/// location (the Shadow Warrior / Shadow Master `tors` and `head` rows):
/// the owner's inventory model is the host's ([`Pending::item_at`]), the
/// code the items row of that item.
pub fn summon_equipment<X: Pending>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    owner: UnitId,
    m: UnitId,
    cutoff: i32,
    ilvl: i32,
) {
    let tables = h.tables.clone();
    let mut host = SummonEquip {
        h,
        sim,
        d,
        store: crate::monsters::init::MonsterStore::default(),
    };
    crate::monsters::init::monequip_rows(
        &tables.monequip,
        &mut host,
        m,
        Some(owner),
        cutoff,
        ilvl,
        false,
    );
}

/// [`crate::monsters::init::InitHost`] for [`summon_equipment`]: only
/// the seed, the class, the holdings and the creation are read.
struct SummonEquip<'a, 'b, X> {
    h: &'a mut ActionHooks<X>,
    sim: &'a mut Sim<'b>,
    d: &'a mut DeathDrops,
    store: crate::monsters::init::MonsterStore,
}

impl<X: Pending> crate::monsters::init::InitHost for SummonEquip<'_, '_, X> {
    fn game(&mut self) -> &mut crate::game::Game {
        self.sim.game
    }
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.h.rules
    }
    fn units(&mut self) -> &mut crate::units::record::Units {
        self.sim.units
    }
    fn monsters(&mut self) -> &mut crate::monsters::init::MonsterStore {
        &mut self.store
    }
    fn info(&self) -> crate::monsters::init::GameInfo {
        crate::monsters::init::GameInfo::default()
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.sim.stats.unit_base(unit, stat, 0)
    }
    /// Not reached by the equipment rules.
    fn set_stat(&mut self, _unit: UnitId, _stat: u16, _value: i32) {}
    fn has_item_at(&mut self, unit: UnitId, loc: u8) -> bool {
        item_at(&*self.h, &*self.sim, unit, loc).is_some()
    }
    /// The owner's item at `loc` from the host's inventory model
    /// ([`Pending::item_at`]), its items-row code.
    fn owner_item_code(&mut self, owner: UnitId, loc: u8) -> Option<[u8; 4]> {
        let item = self.h.x.item_at(owner, loc)?;
        let rec = self.h.items.get(item)?.record;
        Some(self.d.tables.items.item(rec)?.code)
    }
    fn create_equip_item(
        &mut self,
        unit: UnitId,
        code: [u8; 4],
        loc: u8,
        modifier: u8,
        level: i32,
    ) {
        create_monster_equip(
            &mut *self.h,
            &mut *self.sim,
            &mut *self.d,
            unit,
            code,
            loc,
            modifier,
            level,
        );
    }
}

/// The monprop property assignment on a monster: the record
/// `{prop, par, min, max}` writes the stat list with state 0 and flags 0x40 of
/// `unit` (created on demand), the rolls on the unit's own seed.
// PROVISIONAL: mode 0 and format 1 stand in for the caller's
// arguments (the true values are not known); the plain roll
// functions give the right drops with them (checks `items-drops-hel-09` / `-11`,
// the items channel: Hell Mephisto / council member 0xAC drops with
// cast2, swing2, extra-fire).
#[allow(clippy::too_many_arguments)]
pub fn monster_property<X: Pending>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &DeathDrops,
    unit: UnitId,
    prop: i32,
    par: i32,
    min: i32,
    max: i32,
) {
    let t = d.tables.clone();
    let mut fields = GameFields::from_action(
        h.game_seed,
        &h.ai_info,
        sim.data.expansion,
        std::mem::take(&mut h.uniques),
    );
    let mut items = std::mem::take(&mut h.items);
    {
        let mut econ = Economy {
            game: &mut *sim.game,
            units: &mut *sim.units,
            stats: &mut *sim.stats,
            data: sim.data,
            hooks: &mut *h,
            fields: &mut fields,
            tables: &t.items,
            items: &mut items,
        };
        let seed = econ.units.get(unit).map(|r| r.seed);
        let rolled = econ.with_stats(|ctx| {
            let mut item = crate::items::Item::new(0, 1, UnitStats::new(ctx, unit));
            if let Some(s) = seed {
                item.item_seed = s;
            }
            let mut pc = crate::items::props::PropCtx::item(0);
            let rec = crate::items::tables::PropRec {
                code: prop,
                param: par,
                min,
                max,
            };
            crate::items::props::apply_property(&t.items, &mut item, &mut pc, &rec);
            item.item_seed
        });
        if let Some(r) = econ.units.get_mut(unit) {
            r.seed = rolled;
        }
    }
    h.items = items;
    h.uniques = std::mem::take(&mut fields.uniques);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(code: &[u8; 4], uber: &[u8; 4], ultra: &[u8; 4]) -> ItemData {
        ItemData {
            code: *code,
            ubercode: *uber,
            ultracode: *ultra,
            version: 0,
            level: 0,
            type_: 0,
            type2: 0,
            unique: 0,
            quest: 0,
            spawnable: 0,
        }
    }

    #[test]
    fn tier_code_reads_the_first_row_of_the_code() {
        let items = [
            row(b"bsd ", b"9bs ", b"7bs "),
            row(b"bsd ", b"xxx ", b"yyy "),
        ];
        assert_eq!(tier_code(&items, *b"bsd ", 0), *b"bsd ");
        assert_eq!(tier_code(&items, *b"bsd ", 1), *b"9bs ");
        assert_eq!(tier_code(&items, *b"bsd ", 2), *b"7bs ");
        // No row: the code stays on every difficulty.
        assert_eq!(tier_code(&items, *b"zzz ", 2), *b"zzz ");
    }
}
