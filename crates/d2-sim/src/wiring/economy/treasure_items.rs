//! Treasure drops as real items: [`DropSink`] on item creation
//! ([`ItemDrops`]), and the treasure walk's dropper and recipient read from
//! unit records and stat lists. `crate::treasure` runs the walk; this file
//! turns each drop request into an item unit through `Economy::create_item`.

use super::{Economy, EconomyError, ItemSpawn};
use crate::items::ItemRequest;
use crate::stats::StatLists;
use crate::treasure::{DropRequest, DropSink, Dropper, DropperKind, Recipient};
use crate::units::lifecycle::LifecycleHooks;
use crate::units::record::Units;
use crate::units::{RoomId, UnitId, UnitType};

/// Stat ids the walk reads.
const STAT_LEVEL: u16 = 12;
const STAT_GOLD: u16 = 14;
const STAT_GOLDBONUS: u16 = 79;
const STAT_MAGICBONUS: u16 = 80;
const STAT_PLAYERCOUNT: u16 = 100;

/// A drop's room and position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropSpot {
    /// The room the item lands in, if any.
    pub room: Option<RoomId>,
    /// Map x position.
    pub x: i32,
    /// Map y position.
    pub y: i32,
}

/// Seam: the start offset and the free-spot search for a drop. It receives
/// the economy the walk runs on, so a provider reaches the game's rooms and
/// collision through its hooks (`H`).
pub trait DropPlacer<H> {
    /// The spot of the next item dropped by a unit at (`x`, `y`).
    fn place(&mut self, econ: &mut Economy<'_, H>, x: i32, y: i32) -> Option<DropSpot>;

    /// The item `item` was created at `spot`: the provider puts it on the
    /// floor there, so the next search sees it (each search sees the
    /// items placed before it). Default: nothing.
    fn placed(&mut self, _econ: &mut Economy<'_, H>, _item: UnitId, _spot: DropSpot) {}
}

/// The walk's [`DropSink`]: each request becomes a real item unit.
pub struct ItemDrops<'e, 'a, H, P> {
    /// The economy the items are created on.
    pub econ: &'e mut Economy<'a, H>,
    /// Finds the spot for each drop.
    pub placer: P,
    /// Created items with their spots, in creation order (positions
    /// are not stored on the unit).
    pub placed: Vec<(UnitId, DropSpot)>,
    /// Creations that returned no item, with the reason.
    pub failures: Vec<EconomyError>,
    /// The dropper U, used as the request's source unit; none → no source unit.
    pub unit: Option<UnitId>,
}

impl<'e, 'a, H, P> ItemDrops<'e, 'a, H, P> {
    /// A sink with no dropper unit.
    pub fn new(econ: &'e mut Economy<'a, H>, placer: P) -> Self {
        Self {
            econ,
            placer,
            placed: Vec::new(),
            failures: Vec::new(),
            unit: None,
        }
    }

    /// With the dropper U as the request's source unit.
    pub fn with_unit(mut self, unit: UnitId) -> Self {
        self.unit = Some(unit);
        self
    }
}

/// The drop request as an items request; the source unit is the sink's
/// ([`ItemDrops::unit`]).
pub fn drop_request(req: &DropRequest<DropSpot>) -> (ItemRequest, ItemSpawn) {
    let rq = ItemRequest {
        ilvl: req.item_level,
        item: i32::from(req.id),
        format: req.item_format as u16,
        quality: req.quality,
        index: req.index,
        flags2: u32::from(req.drop_flags),
        ..ItemRequest::default()
    };
    let spawn = ItemSpawn {
        room: req.spot.room,
        mode: u32::from(req.spawn_type),
        init_flags: req.init_flags,
    };
    (rq, spawn)
}

impl<H: LifecycleHooks, P: DropPlacer<H>> DropSink for ItemDrops<'_, '_, H, P> {
    type Spot = DropSpot;
    type Item = UnitId;

    fn place(&mut self, x: i32, y: i32) -> Option<DropSpot> {
        self.placer.place(self.econ, x, y)
    }

    fn create(&mut self, req: DropRequest<DropSpot>) -> Option<UnitId> {
        let (mut rq, spawn) = drop_request(&req);
        rq.unit = self.unit.and_then(|u| self.econ.request_unit(u, None));
        match self.econ.create_item(&mut rq, false, spawn) {
            Ok(u) => {
                // A low or normal quality drop without affixes is
                // identified (item creation leaves the flag for the caller to
                // set; no quality or affix step cleared it); magic and
                // better, and charms, stay unidentified.
                // Recorded game drops (S→C 0x9C, flag 0x10): all 14 low / normal items
                // identified, all 12 magic and 2 rare unidentified (the
                // Hellforge Hammer quest item, quality 7, identified); the
                // items channel MATCHes on each.
                if let Some(i) = self.econ.items.get_mut(u) {
                    // A normal charm's affixes cleared it.
                    let affixed = i.prefix.iter().chain(&i.suffix).any(|&a| a != 0);
                    if matches!(i.quality, crate::items::q::LOW | crate::items::q::NORMAL)
                        && !affixed
                    {
                        i.flags |= crate::items::flag::IDENTIFIED;
                    }
                }
                self.placer.placed(self.econ, u, req.spot);
                self.placed.push((u, req.spot));
                Some(u)
            }
            Err(e) => {
                self.failures.push(e);
                None
            }
        }
    }

    /// Stat 14 as stored (the base value; the setter writes the base).
    fn gold(&self, item: UnitId) -> i32 {
        self.econ.stats.unit_base(item, STAT_GOLD, 0)
    }

    fn set_gold(&mut self, item: UnitId, value: i32) {
        self.econ
            .stats
            .unit_set(self.econ.hooks, item, STAT_GOLD, value, 0);
    }
}

/// The dropping unit `U` from its record and stats: a monster's class,
/// `level` (12) and `monster_playercount` (100); a player's base `level`; other units take
/// `area_level` (from the caller: the level generator owns levels).
/// Position comes from the caller.
pub fn dropper(
    units: &Units,
    stats: &StatLists,
    unit: Option<UnitId>,
    area_level: i32,
    x: i32,
    y: i32,
) -> Dropper {
    let kind = match unit.and_then(|u| units.get(u).map(|r| (u, r))) {
        None => DropperKind::None,
        Some((u, r)) => match r.ty {
            UnitType::Monster => DropperKind::Monster {
                class: r.class,
                level: stats.unit_total(u, STAT_LEVEL, 0),
                playercount: stats.unit_total(u, STAT_PLAYERCOUNT, 0),
            },
            UnitType::Player => DropperKind::Player {
                level: stats.unit_base(u, STAT_LEVEL, 0),
            },
            _ => DropperKind::Other { area_level },
        },
    };
    Dropper { kind, x, y }
}

/// The recipient `R` from stats:
/// `M` = stat 80 of `R` plus its minion owner's when `R` is a player or
/// monster, else 0; gold find = stat 79 of `R` plus its owner's. The
/// minion owner and the party count are the caller's.
pub fn recipient(
    units: &Units,
    stats: &StatLists,
    r: UnitId,
    owner: Option<UnitId>,
    party: Option<i32>,
) -> Recipient {
    let sum = |s: u16| {
        let own = stats.unit_total(r, s, 0);
        own.wrapping_add(owner.map_or(0, |o| stats.unit_total(o, s, 0)))
    };
    let fighter = units
        .get(r)
        .is_some_and(|x| matches!(x.ty, UnitType::Player | UnitType::Monster));
    Recipient {
        party,
        magic_find: if fighter { sum(STAT_MAGICBONUS) } else { 0 },
        gold_find: sum(STAT_GOLDBONUS),
    }
}
