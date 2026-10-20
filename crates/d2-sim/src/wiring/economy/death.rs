//! A dead monster's drop: the drop gate and the monster drop run on the
//! action wiring's units, stats and DRLG, with the walk's items created as
//! real item units through [`ItemDrops`] and placed into their rooms.
//!
//! The death start that calls the gate has no body here, so the action wiring
//! hands it to [`Pending::monster_death_start`]; a host that holds the drop
//! state ([`DeathDrops`]) calls [`monster_death_drop`] from there.
//!
//! Real here: the unit flags and the collision word of the gate, the
//! monster's monstats row, the dropper and recipient stats, the walk on the
//! monster's unit seed, the start offset's room search, item creation and
//! allocation on the game seed, into the game's one item store
//! (`ActionHooks::items`, the store every other item system reads: a dropped
//! item can be picked up, sold and cubed). Seams ([`Pending`]): superunique
//! index, champion / unique flags (monster data type flags first,
//! [`ActionHooks::monster_flag`]), minion owner, party, the quest TC test.
//! Position: the path record's ([`ActionHooks::path_position`];
//! [`Pending::position`] without the path provider). The free-spot search:
//! with the path provider on and its walk-back field loaded
//! ([`crate::wiring::path::PathState::field`]), the floor drop
//! [`crate::wiring::path::place::floor_drop`] on the game's rooms, and the
//! created item gets its static path and footprint so the next item of the
//! walk sees it; otherwise the [`FreeSpot`] seam.

use std::sync::Arc;

use d2_data::tables::Superuniques;

use super::{
    dropper, recipient, DropPlacer, DropSpot, Economy, EconomyError, GameFields, ItemDrops,
};
use crate::items::ItemTables;
use crate::path::coords::Point;
use crate::treasure::class_pick::{ClassPicks, PickError};
use crate::treasure::drop::{monster_drop, monster_drop_gate, MonsterDrop, MonsterRank};
use crate::treasure::{ItemData, TreasureClasses, TreasureData, TreasureError};
use crate::units::hooks::Sim;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::action::{ActionHooks, Pending, View, WiringError};
use crate::wiring::path::place::floor_drop;

/// The collision mask the drop gate tests.
const GATE_MASK: u16 = 0x801;
/// Monster type flags: champion and unique.
const FLAG_CHAMPION: u32 = 4;
const FLAG_UNIQUE: u32 = 8;

/// The tables a drop reads beyond the action tables.
#[derive(Debug, Clone)]
pub struct DropTables {
    /// Item tables for creating the dropped items.
    pub items: ItemTables,
    /// Treasure classes.
    pub tcs: TreasureClasses,
    /// [`crate::treasure::item_list`] order (the items' index order).
    pub treasure_items: Vec<ItemData>,
    /// Superunique rows.
    pub superuniques: Vec<Superuniques>,
}

/// A game's drop state: tables, game-creation fields, the host's player
/// counts and what the drops left. The items go to the game's one
/// item store (`ActionHooks::items`).
#[derive(Debug)]
pub struct DeathDrops {
    /// The tables a drop reads.
    pub tables: Arc<DropTables>,
    /// The fields of the last drop, written back after it. Nothing here
    /// is read: the game seed, the creation fields and the unique bits
    /// of a drop are the action wiring's
    /// (`ActionHooks::game_seed`, `ActionHooks::ai_info`,
    /// `UnitData::expansion`, `ActionHooks::uniques`,
    /// [`GameFields::from_action`]), and the seed and the unique bits
    /// go back there (one unique-bit store per game, shared with the
    /// host's economy); `uniques` here is left empty.
    pub fields: GameFields,
    /// The `players` setting, 0–8 ([`DeathDrops::set_players`]). The
    /// living-player count is the game's ([`living_players`]).
    pub players_setting: i32,
    /// Created items with their spots, in creation order.
    pub placed: Vec<(UnitId, DropSpot)>,
    /// Item creations that returned no item.
    pub failures: Vec<EconomyError>,
    /// The walk's fatal errors.
    pub errors: Vec<TreasureError>,
    /// The item class picks of the drop helpers
    /// ([`super::drop_helpers`]): the combined items array's pick columns.
    /// Empty by default (every pick finds no candidate, so the helpers
    /// create nothing).
    pub picks: Arc<ClassPicks>,
    /// The drop helpers' pick fatals.
    pub pick_errors: Vec<PickError>,
}

impl DeathDrops {
    /// A drop state with no placed items or errors yet.
    pub fn new(tables: Arc<DropTables>, fields: GameFields) -> Self {
        Self {
            tables,
            fields,
            players_setting: 0,
            placed: Vec::new(),
            failures: Vec::new(),
            errors: Vec::new(),
            picks: Arc::default(),
            pick_errors: Vec::new(),
        }
    }

    /// The `players` command: values above 8 are ignored.
    pub fn set_players(&mut self, n: i32) {
        if n <= 8 {
            self.players_setting = n;
        }
    }

    /// With the drop helpers' pick rows.
    pub fn with_picks(mut self, picks: Arc<ClassPicks>) -> Self {
        self.picks = picks;
        self
    }
}

/// The living-player count: the players of the game's player list that are
/// not dead.
pub fn living_players(sim: &Sim<'_>) -> i32 {
    sim.game
        .lists
        .units_of_type(UnitType::Player)
        .into_iter()
        .filter(|&u| sim.units.get(u).is_some_and(|r| !r.is_dead()))
        .count() as i32
}

/// Seam: the free-spot search for a dropped item (item size 1, collision
/// mask 0x801), used when the path provider or its walk-back field is off.
/// Items of one walk are placed one after another.
pub trait FreeSpot {
    /// `room` is the dropper's room, never the room the start lookup found.
    fn free_spot(
        &mut self,
        room: Option<RoomId>,
        start: (i32, i32),
        origin: (i32, i32),
    ) -> Option<DropSpot>;

    /// [`Self::free_spot`] with the room the start lookup
    /// found for `start`, for a seam without its own room search (a
    /// D2RUST fill that keeps the start spot needs the start's room).
    /// Default: [`Self::free_spot`], the hint unread.
    fn free_spot_with_start_room(
        &mut self,
        room: Option<RoomId>,
        _start_room: Option<RoomId>,
        start: (i32, i32),
        origin: (i32, i32),
    ) -> Option<DropSpot> {
        self.free_spot(room, start, origin)
    }
}

/// The free-spot search with the start offset done by the caller: `room` is
/// the dropper's room, `start_room` the room the start lookup found (none:
/// the start is the dropper's position).
pub(super) struct Spots<'s, F> {
    /// The seam the search delegates to.
    pub(super) inner: &'s mut F,
    /// The dropper's room.
    pub(super) room: Option<RoomId>,
    /// The room the start lookup found, if any.
    pub(super) start_room: Option<RoomId>,
    /// The start spot after the offset.
    pub(super) start: (i32, i32),
}

/// Item size of the floor drop.
const DROP_SIZE: i32 = 1;

impl<X: Pending, F: FreeSpot> DropPlacer<ActionHooks<X>> for Spots<'_, F> {
    /// With the provider: the floor drop, size 1, fallback 1. Without:
    /// [`FreeSpot`].
    fn place(
        &mut self,
        econ: &mut Economy<'_, ActionHooks<X>>,
        x: i32,
        y: i32,
    ) -> Option<DropSpot> {
        let h = &mut *econ.hooks;
        let Some(field) = h.paths.as_ref().and_then(|p| p.field.clone()) else {
            return self.inner.free_spot_with_start_room(
                self.room,
                self.start_room.or(self.room),
                self.start,
                (x, y),
            );
        };
        match floor_drop(
            &h.drlg,
            &field,
            self.room,
            Point::new(x, y),
            DROP_SIZE,
            true,
        ) {
            Ok((Some(room), p)) => Some(DropSpot {
                room: Some(room),
                x: p.x,
                y: p.y,
            }),
            Ok((None, _)) => None,
            Err(e) => {
                h.errors.push(WiringError::Place(e));
                None
            }
        }
    }

    /// The path part of adding a unit, for the item at its spot (mode 3:
    /// static path and footprint, mask 0x200; without the path provider the
    /// `Pending::place` seam). Also without
    /// the walk-back field: the spot [`FreeSpot`] chose is the item's
    /// position (it was dropped, and the item had no position).
    fn placed(&mut self, econ: &mut Economy<'_, ActionHooks<X>>, item: UnitId, spot: DropSpot) {
        let game: &crate::game::Game = econ.game;
        View::of(econ.units, econ.stats, econ.data, econ.hooks)
            .path_place(game, item, spot.x, spot.y);
        econ.set_ground_expiry(item);
    }
}

/// The gate and the drop of `unit`, killed with death target `target` (the
/// recipient). Returns the created items (also appended to
/// [`DeathDrops::placed`]).
///
/// TODO: without the path provider the collision word at a position outside
/// every room grid is read as 0 (with it: 0x27).
pub fn monster_death_drop<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    spots: &mut F,
    unit: UnitId,
    target: Option<UnitId>,
) -> Vec<UnitId> {
    drop_of(h, sim, d, spots, unit, target, false)
}

/// The Find Item drop: the monster drop with the corpse `corpse` as the
/// dropper (its unit seed), the caster as the recipient and the find-item
/// flag set, without the gate. Returns the created items (also appended to
/// [`DeathDrops::placed`]).
pub fn find_item_drop<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    spots: &mut F,
    corpse: UnitId,
    caster: UnitId,
) -> Vec<UnitId> {
    drop_of(h, sim, d, spots, corpse, Some(caster), true)
}

/// [`monster_death_drop`] (`find_item` false: the gate first) and
/// [`find_item_drop`] (true: no gate, find-item flag set).
fn drop_of<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    spots: &mut F,
    unit: UnitId,
    target: Option<UnitId>,
    find_item: bool,
) -> Vec<UnitId> {
    let Some(r) = sim.units.get(unit) else {
        return Vec::new();
    };
    let (ty, class, flags, mut seed) = (r.ty, r.class, r.flags, r.seed);
    if ty != UnitType::Monster {
        return Vec::new();
    }
    let (x, y) = h.path_position(unit);
    let room = sim.game.lists.unit(unit).and_then(|e| e.room());
    if !find_item {
        // Collision word of the gate: with the path provider, the cell's
        // room among the monster's room and its adjacent rooms; no such
        // room, no record or no grid gives 0x27 (no drop).
        let collision = if h.paths.is_some() {
            crate::path::collision::point_value(&h.drlg, room, x, y, GATE_MASK)
        } else {
            room.and_then(|rm| h.drlg.collision(sim.game, rm, x, y))
                .unwrap_or(0)
        };
        match monster_drop_gate(flags, u32::from(collision & GATE_MASK), class) {
            Ok(true) => {}
            Ok(false) => return Vec::new(),
            Err(e) => {
                d.errors.push(e);
                return Vec::new();
            }
        }
    }
    let at = h.tables.clone();
    let Some(m) = at.combat.monstats.get(class as usize) else {
        return Vec::new();
    };
    let t = d.tables.clone();
    let rank = match h.superunique(unit) {
        Some(i) => MonsterRank::Superunique(t.superuniques.get(usize::from(i))),
        None if h.monster_flag(unit, FLAG_CHAMPION) => MonsterRank::Champion,
        None if h.monster_flag(unit, FLAG_UNIQUE) => MonsterRank::Unique,
        None => MonsterRank::Normal,
    };
    let drop_by = dropper(sim.units, sim.stats, Some(unit), 0, x, y);
    let rec = target.map(|r| {
        recipient(
            sim.units,
            sim.stats,
            r,
            h.minion_owner_of(sim.game, r),
            h.x.party_size(r),
        )
    });
    // The quest test reads quest state only; it is asked up front (it is a
    // query) and handed to the TC choice, which uses it only when its
    // other conditions hold.
    let quest_open = match target {
        // The quest player is the recipient if it is a player, else its
        // minion owner when it has one, else the recipient.
        // TODO: the owner named by a stat list with flag 0x800 is not read.
        Some(r) if m.tcquestid != 0 => {
            let p = match sim.units.get(r).map(|u| u.ty) {
                Some(UnitType::Player) => r,
                _ => h.minion_owner_of(sim.game, r).unwrap_or(r),
            };
            h.x.quest_tc_open(p, m.tcquestid, m.tcquestcp)
        }
        _ => false,
    };
    let start_room = room.and_then(|rm| h.drlg.find_room(sim.game, rm, x + 2, y + 3));
    let start = match start_room {
        Some(_) => (x + 2, y + 3),
        None => (x, y),
    };
    let mut fields = GameFields::from_action(
        h.game_seed,
        &h.ai_info,
        sim.data.expansion,
        std::mem::take(&mut h.uniques),
    );
    let facts = fields.treasure_facts(living_players(sim), d.players_setting);
    let data = TreasureData {
        tcs: &t.tcs,
        items: &t.treasure_items,
        itemtypes: &t.items.itemtypes,
        equiv: &t.items.equiv,
        itemratio: &t.items.itemratio,
    };
    let md = MonsterDrop {
        monstats: m,
        rank,
        find_item,
    };
    // The game's one item store, lent out of the hooks for the drop.
    let mut items = std::mem::take(&mut h.items);
    let (out, placed, failures) = {
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
        let mut sink = ItemDrops::new(
            &mut econ,
            Spots {
                inner: spots,
                room,
                start_room,
                start,
            },
        )
        .with_unit(unit);
        let out = monster_drop(
            &data,
            &facts,
            &md,
            &drop_by,
            &mut seed,
            rec.as_ref(),
            |_| quest_open,
            &mut sink,
        );
        (out, sink.placed, sink.failures)
    };
    h.items = items;
    h.game_seed = fields.seed;
    h.uniques = std::mem::take(&mut fields.uniques);
    d.fields = fields;
    if let Some(r) = sim.units.get_mut(unit) {
        r.seed = seed;
    }
    d.placed.extend(placed);
    d.failures.extend(failures);
    match out {
        Ok(v) => v,
        Err(e) => {
            d.errors.push(e);
            Vec::new()
        }
    }
}
