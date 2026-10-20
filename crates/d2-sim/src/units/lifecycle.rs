//! Unit allocation and removal.
//! The list bookkeeping is [`crate::game::Game`]'s;
//! the per-kind init and free routines beyond the
//! stat list and the missile setup belong to the kind specs and run
//! through [`LifecycleHooks`].

use crate::game::GameError;
use crate::rng::Seed;
use crate::stats::ValueCallback;

use super::hooks::{Sim, UnitHooks};
use super::modes::{self, UnitError};
use super::record::{flags, flags2, UnitRecord, Units};
use super::{ListError, RoomId, UnitId, UnitType};

/// Player classes (step 1: class < 7).
pub const PLAYER_CLASSES: u32 = 7;

/// The allocation arguments (ECX type, EDX class; x, y, game, room,
/// flags, mode, fixed GUID). Positions belong to the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocRequest {
    pub ty: UnitType,
    pub class: u32,
    pub room: Option<RoomId>,
    /// Flags bit 1: `SUNIT_Add`.
    pub add: bool,
    /// Flags bit 2 on a monster: take [`Self::fixed_guid`].
    pub fixed_guid: Option<u32>,
    pub mode: u32,
    /// Counts toward the room's allied count.
    pub allied: bool,
}

/// A call held behind a pending act change ((`ActionHooks::act_tails`)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActTail {
    /// `NpcWorld::act_completion(player, npc, level, from)`.
    Completion {
        player: UnitId,
        npc: UnitId,
        level: u32,
        from: u32,
    },
    /// `NpcWorld::activate_waypoint(player, level)`.
    Waypoint { player: UnitId, level: u32 },
}

/// The per-kind init and free routines other specs own (table).
#[allow(unused_variables)]
pub trait LifecycleHooks: UnitHooks {
    /// The rest of the per-kind init after D2RUST's part (players:
    /// Unless mode 17; monsters; objects: data;
    /// items; tiles), then the path settings of step 8. Provider: the
    /// kind's module.
    fn init_kind(&mut self, sim: &mut Sim<'_>, unit: UnitId, req: &AllocRequest) {}

    /// Step 8's list part ([`add`]) linked `unit`: its per-kind init is
    /// over. Provider: whoever tracks units between steps 7 and 8.
    fn added(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// The quest/NPC act change,
    /// asked from a call that holds no view of the
    /// unit side. Provider: the host that owns the path code, which runs
    /// it right after the call; the default drops it.
    fn request_act_change(&mut self, player: UnitId, level: u32, arg: u32) {}

    /// Holds `tail` behind the player's queued act change; false (the
    /// default) when none is queued, and the caller runs the call now.
    fn hold_act_tail(&mut self, tail: ActTail) -> bool {
        false
    }

    /// The unit's path position (static path +0x0C / +0x10), when it has
    /// a path: a ground item placed by the path code (a treasure drop)
    /// has its position there before the inventory model knows the item.
    /// The default knows none.
    fn path_xy(&self, unit: UnitId) -> Option<(i32, i32)> {
        None
    }

    /// The inventory model put `item` on the ground of `room` at (x, y):
    /// the item's static path follows
    /// (+0x0C / +0x10 are its position), so
    /// the path code (walks to the item, distances) sees where it lies.
    /// The default keeps no paths.
    fn ground_item_placed(&mut self, item: UnitId, room: RoomId, x: i32, y: i32) {}

    /// A ground item leaves the ground (the collision free
    /// of the pickups): its footprint is
    /// cleared. The default keeps no paths.
    fn ground_item_lifted(&mut self, item: UnitId) {}

    /// A unit leaves a room's list (room delete record):
    /// the clients in the room get its RemoveUnit in the next flush, once
    /// per unit. The default records nothing.
    fn room_delete_record(&mut self, game: &mut crate::game::Game, guid: u32, room: RoomId) {}

    /// The room lies in a town level. The default knows no
    /// levels.
    fn town_room(&self, game: &crate::game::Game, room: RoomId) -> bool {
        false
    }

    /// The level id of the room. The default knows no levels.
    fn room_level(&self, game: &crate::game::Game, room: RoomId) -> Option<u32> {
        None
    }

    /// The Town Portal cast,
    /// word 1 of item-use entry 2,
    /// asked by the item use of a call that holds
    /// the units and the game seed (`seed`: the call's game seed, drawn by
    /// the portal objects' allocation): the result (1 made,
    /// 0 not) and whether the cast's own S→C 0x7C for the item is due
    /// (step 9). Provider: the host that owns the objects and the path
    /// code; `None` (the default): no such host (the desk then records a
    /// request, `wiring/inventory/town_portal.rs`).
    fn town_portal_cast(
        &mut self,
        sim: &mut Sim<'_>,
        seed: &mut Seed,
        player: UnitId,
    ) -> Option<(u32, bool)> {
        None
    }

    /// The free routine's other calls (per kind) at removal. Provider:
    /// the kind.
    fn free_kind(&mut self, sim: &mut Sim<'_>, unit: UnitId) {}

    /// The monster of `class` created near `near`
    /// (the spawn `(game, near, class, mode, 4, 0)`, step
    /// 7: the mercenary's creation), its allocation
    /// drawing `seed` (the game seed). Provider: the host
    /// that owns the path code; `None` (the default) leaves the creation
    /// to the NPC rest.
    fn spawn_near(
        &mut self,
        sim: &mut Sim<'_>,
        seed: &mut Seed,
        near: UnitId,
        class: u32,
        mode: u8,
    ) -> Option<UnitId> {
        None
    }

    /// [`Self::spawn_near`] with the spawn's spread (the quest
    /// mercenary's 4, 6, 12). Default: spread 4 only.
    fn spawn_near_spread(
        &mut self,
        sim: &mut Sim<'_>,
        seed: &mut Seed,
        near: UnitId,
        class: u32,
        mode: u8,
        spread: i32,
    ) -> Option<UnitId> {
        if spread == 4 {
            self.spawn_near(sim, seed, near, class, mode)
        } else {
            None
        }
    }

    /// Owner data with (game, unit, GUID, type, …) on the unit's AI
    /// control record (the minion owner), as the hireling creation's step 8 links a
    /// mercenary to its player. Provider: the host that holds the AI
    /// store; the default does nothing.
    fn set_ai_owner(&mut self, unit: UnitId, owner_type: u8, owner_guid: u32) {}

    /// The removal notice `(room, type, GUID)` of a unit freed by the host's NPC
    /// calls (the hireling revive's step 3): the
    /// room's removal notice, sent to the clients in the next per-client
    /// update. Default: nothing.
    fn queue_removal_notice(&mut self, unit_type: u8, guid: u32) {}

    /// A stat record on the unit's message
    /// list (+0xEC), flushed to each updating client.
    /// Provider: the host with the unit
    /// event records; the default does nothing.
    fn queue_unit_stat(&mut self, unit: UnitId, stat: u16, value: u32) {}

    /// The umod appended to the
    /// monster's list and its init run (create
    /// `assign_umod`), as the hireling creation's step 10 gives a mercenary
    /// umod 19. Provider: the host that holds the monster state; the
    /// default does nothing.
    fn assign_umod(&mut self, sim: &mut Sim<'_>, unit: UnitId, umod: u8) {}

    /// The monster data bytes clears after the umod
    /// (monster data +4, +5, +9, +10, +11,
    /// i.e. components 0, 1, 5, 6, 7; asm–).
    /// Provider: the host that holds the monster state; the default does
    /// nothing.
    fn clear_hireling_components(&mut self, unit: UnitId) {}

    /// The alignment stat in the unit's
    /// state 105 list, created when missing.
    /// Provider: the host with the
    /// state-list view; the default does nothing.
    fn set_alignment(&mut self, sim: &mut Sim<'_>, unit: UnitId, value: u8) {}

    /// Warp of one pet to its player (the hireling warp).
    /// Provider: the host that owns the
    /// path code; the default does nothing.
    fn warp_pet(&mut self, sim: &mut Sim<'_>, pet: UnitId, player: UnitId) {}

    /// The `skills` table (count `+0xBA0` and `reqlevel` +0x174) as the
    /// hireling level rule reads it.
    /// Provider: the host holding the skill tables; `None` (the default)
    /// leaves the answer to the hireling rest.
    fn skill_tables(&self) -> Option<&crate::skills::SkillTables> {
        None
    }

    /// The rules the game runs under; the
    /// items formulas read a mod game's functions from them. Default:
    /// D2RUST's own.
    fn rules(&self) -> &crate::rules::Ruleset {
        &crate::rules::ORIGINAL
    }

    /// The skill set `(unit, skill, level)` on a monster (the
    /// mercenary's row skills): the entry in
    /// the monster's skill list. Provider: the host holding the monster
    /// skill entries; `false` (the default) leaves it to the hireling
    /// rest.
    fn set_monster_skill_level(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        skill: i32,
        level: i32,
    ) -> bool {
        false
    }
}

/// [`allocate_unlinked`] (steps 1–7), then
/// [`add`] (step 8's `SUNIT_Add` list part). Returns `None` when the
/// class is rejected (nothing allocated, no RNG draw). `game_seed` is
/// the game seed. An error (unknown room, duplicate
/// GUID) leaves the game, the seed and the GUID counter as they were.
pub fn allocate<H: LifecycleHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    game_seed: &mut Seed,
    req: &AllocRequest,
) -> Result<Option<UnitId>, UnitError> {
    let Some(unit) = allocate_unlinked(sim, hooks, game_seed, req)? else {
        return Ok(None);
    };
    // Flags bit 1 clear → no `SUNIT_Add`; the unit is returned
    // as it is after step 7, in no list.
    if req.add {
        add(sim, hooks, unit, req)?;
    }
    Ok(Some(unit))
}

/// `SUNIT_InitSeed` with the game seed as
/// parent: one game-seed step `lo'`; the unit seed is `init_low(lo')`
/// and `dwInitSeed` (+0x28) is `lo'`. Returns both.
pub fn draw_unit_seed(game_seed: &mut Seed) -> (Seed, u32) {
    let lo = game_seed.step();
    (Seed::init_low(lo), lo)
}

/// The player's unit seed: run on an allocated
/// player by its load, on the game seed. Unknown unit: no draw.
pub fn init_player_seed(units: &mut Units, unit: UnitId, game_seed: &mut Seed) -> bool {
    let Some(rec) = units.get_mut(unit) else {
        return false;
    };
    (rec.seed, rec.init_seed) = draw_unit_seed(game_seed);
    true
}

/// Step 8's list part: `SUNIT_Add`
/// of a unit [`allocate_unlinked`] returned, in the allocation's room.
/// The path settings of step 8 are the path provider's.
pub fn add<H: LifecycleHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
    req: &AllocRequest,
) -> Result<(), UnitError> {
    sim.game
        .lists
        .link_unit(unit, req.room)
        .map_err(GameError::from)?;
    hooks.added(sim, unit);
    Ok(())
}

/// Steps 1–7 of the unit creation: the unit with its seeds, GUID and
/// per-kind init, in no list ([`super::UnitLists::reserve_unit`]):
/// an init never finds the unit by a hash lookup and reads its
/// room from the allocation's argument. The GUID is tested free
/// here, so step 8 cannot meet the fatal duplicate
/// after the init ran; that error undoes the draws of steps 4
/// and 6.
pub fn allocate_unlinked<H: LifecycleHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    game_seed: &mut Seed,
    req: &AllocRequest,
) -> Result<Option<UnitId>, UnitError> {
    // Step 1.
    match req.ty {
        UnitType::Monster if !sim.data.monster(req.class).is_some_and(|m| m.enabled) => {
            return Ok(None)
        }
        UnitType::Player if req.class >= PLAYER_CLASSES => return Ok(None),
        _ => {}
    }
    // Step 3: the act of the allocation room's level. An unknown room
    // is refused here, before any draw.
    let act = match req.room {
        Some(r) => {
            sim.game
                .lists
                .room(r)
                .ok_or(ListError::UnknownRoom(r))
                .map_err(GameError::from)?
                .act
        }
        None => 0,
    };
    // A refused `SUNIT_Add` (the fatal duplicate GUID of the unit
    // order) undoes the draws of steps 4 and 6.
    let undo = (*game_seed, sim.game.lists.guids.get(req.ty));
    // Step 4.
    let mut seed = Seed::init();
    let mut init_seed = 0;
    let mut item_seed = None;
    if req.ty != UnitType::Player {
        (seed, init_seed) = draw_unit_seed(game_seed);
    }
    if req.ty == UnitType::Item {
        let start = game_seed.step();
        item_seed = Some((Seed::init_low(start), start));
    }
    // Step 6.
    let guid = match (req.ty, req.fixed_guid) {
        (UnitType::Monster, Some(g)) => g,
        _ => sim.game.lists.guids.alloc(req.ty),
    };
    // The entry exists from here (it owns timers) but is in no list
    // until step 8.
    // Without `SUNIT_Add` (flags bit 1 clear) no list is touched,
    // so a taken GUID is not met either.
    let free = if req.add {
        sim.game.lists.check_guid_free(req.ty, guid)
    } else {
        Ok(())
    };
    if let Err(e) = free {
        *game_seed = undo.0;
        sim.game.lists.guids.set(req.ty, undo.1);
        return Err(GameError::from(e).into());
    }
    let unit = sim.game.lists.reserve_unit(req.ty, guid, req.allied);
    // Steps 2 and 5.
    let mut rec = UnitRecord::new(req.ty, req.class, guid);
    rec.act = act;
    rec.seed = seed;
    rec.init_seed = init_seed;
    rec.item_seed = item_seed;
    rec.flags |= flags::SEED_SET;
    // Flag 0x2 (a skill target): the tile init sets it, and
    // so does the item init (clearing 0x8, never set here),
    // so every item starts targetable (table).
    if matches!(req.ty, UnitType::Tile | UnitType::Item) {
        rec.flags |= flags::TILE;
    }
    rec.flags2 |= flags2::SERVER;
    if sim.data.expansion {
        rec.flags2 |= flags2::EXPANSION;
    }
    // Step 7 (table): mode := argument for monsters, objects, missiles
    // and items; the tile init (flags |= 0x2) sets no mode.
    if !matches!(req.ty, UnitType::Player | UnitType::Tile) {
        rec.mode = req.mode;
    }
    sim.units.insert(unit, rec);
    // Step 7: the stat lists and the missile setup.
    let callback = match req.ty {
        UnitType::Player | UnitType::Monster => Some(Some(ValueCallback::Server)),
        UnitType::Item | UnitType::Missile => Some(None),
        _ => None,
    };
    if let Some(cb) = callback {
        let l = sim
            .stats
            .alloc_extended(hooks, unit, req.ty, guid, req.class, 0, cb);
        if let Some(r) = sim.units.get_mut(unit) {
            r.stats = Some(l);
        }
    }
    if req.ty == UnitType::Missile {
        modes::missile_init(sim, unit)?;
    }
    hooks.init_kind(sim, unit, req);
    Ok(Some(unit))
}

/// List unlinks and timer cancels
/// ([`crate::game::Game::remove_unit`]), the kind's free routine, the
/// stat list, the record. Immediate.
pub fn remove<H: LifecycleHooks>(
    sim: &mut Sim<'_>,
    hooks: &mut H,
    unit: UnitId,
) -> Result<(), UnitError> {
    sim.game.remove_unit(unit)?;
    hooks.free_kind(sim, unit);
    sim.stats.free_unit_list(hooks, unit);
    sim.units.remove(unit);
    Ok(())
}
