//! Server missiles: the per-missile data, creation, the
//! per-tick dispatch, the default flight, the hit handler and
//! damage stage, lifetime, pierce and the server-do /
//! server-hit catalogues ([`catalogue`]).
//!
//! Units, paths, rooms, collision, stats, damage and the skill code live
//! in other modules. They are reached through the
//! narrow traits of [`seams`]; [`MissileWorld`] bundles them. Missile
//! timer events plug into the tick through [`MissileDispatch`], an
//! [`EventDispatch`] that runs the missile class handler and hands every
//! other event to the next dispatcher.

pub mod bodies;
pub mod bodies_ext;
pub mod bodies_ext2;
pub mod catalogue;
mod create;
pub mod damage;
mod flight;
mod hit;
pub mod init_cb;
pub mod provider;
pub mod seams;
pub mod srv_dmg;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_bodies;

use std::collections::BTreeMap;

use d2_data::tables::Missiles as MissileRow;

use crate::game::Game;
use crate::tick::timer::{TimerClass, TimerRun};
use crate::tick::EventDispatch;
use crate::units::{UnitId, UnitType};

pub use create::{
    create_missile, creation_velocity, frames_from_distance, pierce_count, MissileParams,
};
pub use flight::{default_flight, PathVelocity};
pub use hit::{
    damage_roll, damage_tail, fill_damage, hit_handler, pct, result_flag, result_flags, Damage,
    ELEMENTS,
};
pub use provider::{MissileProvider, MissileSlot};
pub use seams::{
    AsDynMissile, MissileBodies, MissileCombat, MissileHooks, MissilePath, MissileRooms,
    MissileUnits, MissileWorld, SkillCalc, SkillField, SummonClass,
};

/// Flag bits of the parameter record read by
/// creation.
pub mod param_flags {
    /// Position (x, y) given; else the origin's position.
    pub const POSITION: u32 = 0x1;
    /// Target (x, y) relative to the start.
    pub const TARGET_RELATIVE: u32 = 0x2;
    /// Velocity given.
    pub const VELOCITY: u32 = 0x4;
    /// Add loops × (SubStop − SubStart) frames.
    pub const LOOPS: u32 = 0x8;
    /// Given velocity is already fixed point.
    pub const VELOCITY_FIXED: u32 = 0x10;
    /// Target (x, y) absolute.
    pub const TARGET_ABSOLUTE: u32 = 0x20;
    /// Start frame given.
    pub const START_FRAME: u32 = 0x200;
    /// Current frame from the distance to the target.
    pub const FRAMES_FROM_DISTANCE: u32 = 0x400;
    /// Activate frames given.
    pub const ACTIVATE: u32 = 0x800;
    /// Attack bonus → stat 19.
    pub const ATTACK_BONUS: u32 = 0x1000;
    /// Range given.
    pub const RANGE: u32 = 0x8000;
    /// Missile data flag 2.
    pub const DATA_FLAG_2: u32 = 0x10000;
}

/// Stat ids the missile code reads or writes (`itemstatcost.txt` ids,
/// missile constants).
pub mod stat {
    pub const TOHIT: u16 = 19;
    pub const MINDAMAGE: u16 = 21;
    pub const MAXDAMAGE: u16 = 22;
    pub const DAMAGEPERCENT: u16 = 25;
    pub const FIREMINDAM: u16 = 48;
    pub const FIREMAXDAM: u16 = 49;
    pub const LIGHTMINDAM: u16 = 50;
    pub const LIGHTMAXDAM: u16 = 51;
    pub const MAGICMINDAM: u16 = 52;
    pub const MAGICMAXDAM: u16 = 53;
    pub const COLDMINDAM: u16 = 54;
    pub const COLDMAXDAM: u16 = 55;
    pub const COLDLENGTH: u16 = 56;
    pub const POISONMINDAM: u16 = 57;
    pub const POISONMAXDAM: u16 = 58;
    pub const POISONLENGTH: u16 = 59;
    pub const LIFEDRAINMINDAM: u16 = 60;
    pub const MANADRAINMINDAM: u16 = 62;
    pub const STAMDRAINMINDAM: u16 = 64;
    pub const STUNLENGTH: u16 = 66;
    pub const IGNORE_TARGET_AC: u16 = 103;
    pub const FRACTIONAL_TARGET_AC: u16 = 104;
    pub const IGNORE_TARGET_DEFENSE: u16 = 106;
    pub const ITEM_DAMAGETARGETAC: u16 = 120;
    pub const DEADLY_STRIKE: u16 = 141;
    pub const ITEM_PIERCE: u16 = 156;
    pub const SKILL_HANDOFATHENA: u16 = 161;
    pub const SKILL_PIERCE: u16 = 166;
    pub const BURNINGMINDAM: u16 = 316;
    pub const BURNINGMAXDAM: u16 = 317;
    pub const BURNINGLENGTH: u16 = 315;
    pub const POISON_COUNT: u16 = 326;
    pub const DAMAGE_FRAMERATE: u16 = 327;
    pub const PIERCE_IDX: u16 = 328;
    pub const FIRE_MASTERY: u16 = 329;
    pub const LIGHT_MASTERY: u16 = 330;
    pub const COLD_MASTERY: u16 = 331;
    pub const POISON_MASTERY: u16 = 332;
    pub const MAGIC_MASTERY: u16 = 357;
}

/// State ids (`states.txt` rows).
pub mod state {
    pub const UNINTERRUPTABLE: u16 = 54;
    pub const JUSTHIT: u16 = 86;
    pub const SLOWMISSILES: u16 = 87;
}

/// Unit flag bits the missile code touches.
pub mod unit_flag {
    /// Bit 1, cleared at missile init.
    pub const BIT1: u32 = 1 << 1;
    /// Bit 2: set during creation when `CanDestroy` is enabled.
    pub const CAN_BE_ATTACKED: u32 = 1 << 2;
    /// Bit 3: required by the flight collision filter for a unit target.
    pub const IS_VALID_TARGET: u32 = 1 << 3;
}

/// Collision mask bits (the unit collision masks).
pub mod coll {
    pub const WALL: u16 = 0x1;
    pub const MISSILE_BARRIER: u16 = 0x4;
    pub const FOOTPRINT: u16 = 0x40;
    pub const PLAYER: u16 = 0x80;
    pub const MONSTER: u16 = 0x100;
}

/// Server-do table size.
pub const SRV_DO_COUNT: i16 = 53;
/// Server-hit table size.
pub const SRV_HIT_COUNT: i16 = 71;
/// Server-damage table size.
pub const SRV_DMG_COUNT: i16 = 31;

/// Unit categories accepted by a collide type's callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accept {
    /// Players, and good-aligned monsters.
    PlayersAndGoodMonsters,
    /// Monsters.
    Monsters,
    /// Players and monsters.
    PlayersAndMonsters,
    /// Missiles with `CanDestroy`.
    DestroyableMissiles,
}

/// One entry of the collide-type table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollideMode {
    pub callback: Option<Accept>,
    pub mask: u16,
}

/// Table: 9 (callback, mask) entries, by missile mode
/// (= `CollideType`).
pub const COLLIDE_MODES: [CollideMode; 9] = [
    CollideMode {
        callback: None,
        mask: 0,
    },
    CollideMode {
        callback: Some(Accept::PlayersAndGoodMonsters),
        mask: 0x84,
    },
    CollideMode {
        callback: Some(Accept::Monsters),
        mask: 0x104,
    },
    CollideMode {
        callback: Some(Accept::PlayersAndMonsters),
        mask: 0x184,
    },
    CollideMode {
        callback: None,
        mask: 0,
    },
    CollideMode {
        callback: Some(Accept::Monsters),
        mask: 0x104,
    },
    CollideMode {
        callback: None,
        mask: 0x4,
    },
    CollideMode {
        callback: Some(Accept::DestroyableMissiles),
        mask: 0x40,
    },
    CollideMode {
        callback: Some(Accept::PlayersAndMonsters),
        mask: 0x185,
    },
];

/// The collide-type entry for modes 0–8. Returns `None` outside the table;
/// flight and creation use a missing entry as no callback and mask 0.
pub fn collide_mode(mode: u8) -> Option<CollideMode> {
    COLLIDE_MODES.get(mode as usize).copied()
}

/// A unit by (type, GUID): how the missile stores its owner and its
/// last-collided unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitRef {
    pub ty: UnitType,
    pub guid: u32,
}

/// Per-missile state: class, collision mode, lifetime, skill, owner,
/// last-collided unit and handler-specific target fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissileData {
    /// `missiles.txt` row (unit class).
    pub class: u16,
    /// Unit mode = the row's `CollideType`.
    pub mode: u8,
    /// Activate frame.
    pub activate: i16,
    /// Skill (clamped 0…0x7FFF by its setter).
    pub skill: i16,
    /// Level (stored unclamped, truncated to 16 bits).
    pub level: i16,
    /// Total frames.
    pub total: i16,
    /// Current frame = frames left.
    pub current: i16,
    /// Flags.
    pub flags: u32,
    /// Last-collided unit (GUID −1 = `None`).
    pub last_collided: Option<UnitRef>,
    /// Target fields: free state of server-do functions.
    pub target: (i32, i32),
    /// Owner stored at creation.
    pub owner: Option<UnitRef>,
}

impl MissileData {
    fn new(class: u16, mode: u8) -> Self {
        Self {
            class,
            mode,
            activate: 0,
            skill: 0,
            level: 0,
            total: 0,
            current: 0,
            flags: 0,
            last_collided: None,
            target: (0, 0),
            owner: None,
        }
    }

    /// "Elapsed frames": total − current.
    pub fn elapsed(&self) -> i32 {
        i32::from(self.total) - i32::from(self.current)
    }
}

/// The frame setters' clamp:
/// −0x8000…0x7FFF.
pub fn clamp_frame(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// Logged dispatch cases without a handler and fatal conditions that stop
/// the current missile function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unhandled {
    /// Server-do function `index` (stub; `srvdo.tsv`).
    SrvDo { index: i16, missile: UnitId },
    /// Server-hit function `index` (stub; `srvhit.tsv`).
    SrvHit { index: i16, missile: UnitId },
    /// Dispatch reached a null server-do entry.
    NullSrvDo { index: i16, missile: UnitId },
    /// Dispatch reached a null server-hit entry.
    NullSrvHit { index: i16, missile: UnitId },
    /// Dispatch reached a null server-damage entry (15–30).
    NullSrvDmg { index: i16, missile: UnitId },
    /// A guard stopped the function identified by `addr`; the rest of that
    /// function is skipped.
    Fatal { addr: u32, missile: UnitId },
}

/// The missile system's state: per-missile data by unit, plus the log of
/// unmodelled calls.
#[derive(Clone, Debug, Default)]
pub struct MissileStore {
    data: BTreeMap<UnitId, MissileData>,
    /// Calls into stubs and null entries, in order.
    pub unhandled: Vec<Unhandled>,
}

impl MissileStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, m: UnitId) -> Option<&MissileData> {
        self.data.get(&m)
    }

    pub fn get_mut(&mut self, m: UnitId) -> Option<&mut MissileData> {
        self.data.get_mut(&m)
    }

    /// Drops a missile's data when the unit is removed by code other
    /// than the missile handler (unit removal frees it with
    /// the missile free).
    pub fn remove(&mut self, m: UnitId) -> Option<MissileData> {
        self.data.remove(&m)
    }

    /// Missiles with data, in unit-slot order.
    pub fn missiles(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.data.keys().copied()
    }
}

/// What missile code works with: the `missiles.txt` records, the store
/// and the other systems.
pub struct Ctx<'a, W: ?Sized> {
    pub tables: &'a [MissileRow],
    pub store: &'a mut MissileStore,
    pub world: &'a mut W,
}

impl<W: MissileWorld + ?Sized> Ctx<'_, W> {
    /// The same context with the world as `dyn` [`MissileWorld`], for a
    /// mod game's bodies.
    pub fn as_dyn(&mut self) -> Ctx<'_, dyn MissileWorld + '_> {
        Ctx {
            tables: self.tables,
            store: &mut *self.store,
            world: self.world.as_dyn_missile(),
        }
    }

    /// The `missiles.txt` record of a class (range-checked).
    pub fn row(&self, class: i32) -> Option<&MissileRow> {
        usize::try_from(class).ok().and_then(|c| self.tables.get(c))
    }

    /// The record of a missile's class.
    pub(crate) fn row_of(&self, m: UnitId) -> Option<&MissileRow> {
        let class = self.store.get(m)?.class;
        self.row(i32::from(class))
    }

    /// `SUNIT_GetOwner`: the stored owner if it still
    /// exists.
    pub fn owner(&self, game: &Game, m: UnitId) -> Option<UnitId> {
        let o = self.store.get(m)?.owner?;
        game.lists.find_unit(o.ty, o.guid)
    }

    /// "Remove": frees the unit and cancels its
    /// timers; the missile free drops the missile data.
    pub fn remove(&mut self, game: &mut Game, m: UnitId) {
        self.world.remove_unit(game, m);
        self.store.data.remove(&m);
    }
}

/// Signed 16-bit reads of a record.
pub trait RowExt {
    fn srv_do(&self) -> i16;
    fn srv_hit(&self) -> i16;
    fn srv_dmg(&self) -> i16;
    fn range_i16(&self) -> i16;
    fn lev_range_i16(&self) -> i16;
    fn accel_i16(&self) -> i16;
}

impl RowExt for MissileRow {
    fn srv_do(&self) -> i16 {
        self.psrvdofunc as i16
    }
    fn srv_hit(&self) -> i16 {
        self.psrvhitfunc as i16
    }
    fn srv_dmg(&self) -> i16 {
        self.psrvdmgfunc as i16
    }
    fn range_i16(&self) -> i16 {
        self.range as i16
    }
    fn lev_range_i16(&self) -> i16 {
        self.levrange as i16
    }
    fn accel_i16(&self) -> i16 {
        self.accel as i16
    }
}

/// Missile initialisation, called inside unit allocation
/// (creation step 9) right after the unit exists: missile data, unit flags 1
/// type-0 event scheduled (args 0, 0).
pub fn init_missile<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    class: u16,
    mode: u8,
) {
    cx.store.data.insert(m, MissileData::new(class, mode));
    cx.world.set_unit_flag(m, unit_flag::BIT1, false);
    cx.world.set_unit_flag(m, unit_flag::IS_VALID_TARGET, false);
    game.timers.cancel_unit_timers(m);
    if let Ok(owner) = game.owner(m) {
        // A missile always has a timer class: the error is unreachable.
        let _ = game.timers.schedule_every_tick(owner, 0, None, 0, 0);
    }
}

/// The missile class default handler:
/// event type and arguments are ignored.
pub fn class_handler<W: MissileWorld + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, m: UnitId) {
    // Step 1.
    let Some(row) = cx.row_of(m) else {
        return;
    };
    let srv_do = row.srv_do();
    let (src_town, town) = (row.srctown, row.town);
    if !catalogue::srv_do_called(cx, srv_do) {
        return;
    }
    // Step 2: no room → the null-start lookup always yields none.
    let room = game.lists.unit(m).and_then(|e| e.room());
    // Step 3.
    let owner = cx.owner(game, m);
    let owner_room = owner.and_then(|o| game.lists.unit(o).and_then(|e| e.room()));
    let owner_in_town = owner_room.is_some_and(|r| cx.world.in_town(game, r));
    // Step 4.
    if src_town && owner_in_town {
        cx.remove(game, m);
        return;
    }
    // Step 5: a player owner in town removes any missile.
    let owner_is_player = owner
        .and_then(|o| game.lists.unit(o))
        .is_some_and(|e| e.ty == UnitType::Player);
    if owner_is_player && owner_in_town {
        cx.remove(game, m);
        return;
    }
    // Step 6.
    if !town && room.is_some_and(|r| cx.world.in_town(game, r)) {
        cx.remove(game, m);
        return;
    }
    // Step 7.
    if catalogue::run_srv_do(game, cx, srv_do, m) == 2 {
        cx.remove(game, m);
    }
}

/// [`EventDispatch`] for the tick: missile-class events without an
/// explicit callback run [`class_handler`]; everything else goes to
/// `next`.
pub struct MissileDispatch<'a, W: ?Sized, N: ?Sized> {
    pub cx: Ctx<'a, W>,
    pub next: &'a mut N,
}

impl<W: MissileWorld + ?Sized, N: EventDispatch + ?Sized> EventDispatch
    for MissileDispatch<'_, W, N>
{
    fn run_event(&mut self, game: &mut Game, run: &TimerRun) {
        if run.class == TimerClass::Missile && run.callback.is_none() {
            class_handler(game, &mut self.cx, run.owner.unit);
        } else {
            self.next.run_event(game, run);
        }
    }
}
