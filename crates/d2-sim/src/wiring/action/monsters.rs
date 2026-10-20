//! The monster side of a game as the action hooks reach it: a
//! [`MonsterWorld`] lent to [`ActionHooks::monster_world`] (the
//! world-generation state, `wiring::worldgen::WorldState`, implements it;
//! `WorldSim` lends it around its timer events and tick hooks). With it:
//!
//! - a monster the allocator makes gets its type init from the per-kind init
//!   hook, wherever the allocation comes from;
//! - a removed unit leaves the monster state too;
//! - the umod dispatcher runs for monster timer event 7 (mode 2), the
//!   monster mode change (modes 0 and 1), the combat hook (mode 3) and the
//!   missile hook (mode 5);
//! - the monster-data queries (type flags, the monster level) read the
//!   monster data.
//!
//! Without a world (an [`super::ActionSim`] alone, or while the world is
//! lent to a call that holds it directly) every one of these keeps its
//! [`Pending`] answer, as before.

use std::any::Any;

use crate::monsters::init::MonsterData;
use crate::units::hooks::Sim;
use crate::units::{RoomId, UnitId};

use super::{ActionHooks, Pending, WiringError};

/// Umod dispatcher modes: the situation in which the dispatcher runs a monster's umods.
pub mod umod_mode {
    /// The monster mode change.
    pub const MODE_CHANGE: u8 = 0;
    /// The monster mode change (second call site).
    pub const MODE_SET: u8 = 1;
    /// Monster timer event 7.
    pub const EVENT7: u8 = 2;
    /// A combat hit.
    pub const HIT: u8 = 3;
    /// The hit reaction. Its sites are inside the monster-defender branches
    /// of the reaction, which stay [`super::Pending::reaction`]'s: the
    /// provider of those branches runs it.
    pub const GET_HIT: u8 = 4;
    /// Missile creation.
    pub const MISSILE: u8 = 5;
}

/// The monster state of a game (monster data, umods, minion and owner
/// links, monster init), lent to the action hooks. The calls get the
/// action hooks with the world taken out of them, so the two never
/// alias.
pub trait MonsterWorld<X> {
    /// Monster type init of a monster the allocator just made (the
    /// allocator's last step).
    fn type_init(&mut self, sim: &mut Sim<'_>, h: &mut ActionHooks<X>, unit: UnitId);
    /// The umod dispatcher: runs `unit`'s umods for `mode` (`arg` is the other
    /// unit involved, if any).
    fn umods(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        arg: Option<UnitId>,
        mode: u8,
    );
    /// Appends `umod` to the unit's list and runs its init.
    fn assign_umod(&mut self, sim: &mut Sim<'_>, h: &mut ActionHooks<X>, unit: UnitId, umod: u8);
    /// As [`Self::assign_umod`] with the "unique" argument (marks the monster
    /// unique). Default: the plain assign.
    fn assign_umod_arg(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        umod: u8,
        unique: bool,
    ) {
        let _ = unique;
        self.assign_umod(sim, h, unit, umod);
    }
    /// The monster state's part of a unit free.
    fn forget(&mut self, unit: UnitId);
    /// The monster data of `unit`, if it has one.
    fn monster(&self, unit: UnitId) -> Option<&MonsterData>;
    /// The classes of the first 16 entries of the level's monster region
    /// (used for the object trap's monster id). Default: no region.
    fn region_classes(&self, level: u32) -> Option<Vec<i32>> {
        let _ = level;
        None
    }
    /// The `mon1…` list of the levels record of `level`. `None`: no such
    /// record. Default: none.
    fn level_mon_list(&self, level: u32) -> Option<Vec<i32>> {
        let _ = level;
        None
    }
    /// The monstats `isSpawn` flag of `class`; `None` for a class with no
    /// record. Default: none.
    fn class_is_spawn(&self, class: i32) -> Option<bool> {
        let _ = class;
        None
    }
    /// Called from the death start: counts the dying monster in its region's
    /// kill count; `alignment` is its alignment. Default: nothing.
    fn count_death(&mut self, unit: UnitId, alignment: u8) {
        let _ = (unit, alignment);
    }
    /// Level 8's region for the Den of Evil quest: (evil spawned, evil killed,
    /// rooms populated, total population rooms). Default: no region.
    fn den_counts(&self) -> Option<(u32, u32, u32, u32)> {
        None
    }
    /// Class reinit: turns `unit` into `class` in `mode`. Default: nothing
    /// (false).
    fn reinit(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        class: i32,
        mode: u32,
    ) -> bool {
        let _ = (sim, h, unit, class, mode);
        false
    }
    /// The `monstats` row count. Default 0.
    fn monstats_count(&self) -> u32 {
        0
    }
    /// The monster data of `unit`, mutable. Default: none.
    fn monster_mut(&mut self, _unit: UnitId) -> Option<&mut MonsterData> {
        None
    }
    /// The choice counts of the class's 16 components (monstats2 of
    /// `MonStatsEx`). Default: none.
    fn component_counts(&self, _class: u32) -> Option<[u8; 16]> {
        None
    }
    /// Creates a monster in `room` near (x, y) on the lent world: placement
    /// on the room seed, the allocation with its type init, alignment, normal
    /// and boss mods, party. `None`: the world cannot run it (the caller
    /// allocates plainly); `Some(None)`: nothing placed.
    #[allow(clippy::too_many_arguments)]
    fn spawn_at(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: RoomId,
        x: i32,
        y: i32,
        class: i32,
        mode: u8,
        spread: i32,
        flags: u16,
    ) -> Option<Option<UnitId>> {
        let _ = (sim, h, room, x, y, class, mode, spread, flags);
        None
    }
    /// The plain-record restore creation, with the record's GUID, on the lent
    /// world: spreads -1 and 5, the spawn point, the nearest free point.
    /// `None`: the world cannot run it; `Some(None)`: nothing placed.
    #[allow(clippy::too_many_arguments)]
    fn restore_spawn(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: RoomId,
        x: i32,
        y: i32,
        guid: u32,
        class: i32,
        mode: u8,
        flags: u16,
    ) -> Option<Option<UnitId>> {
        let _ = (sim, h, room, x, y, guid, class, mode, flags);
        None
    }
    /// The spawn point for `class` in `room` on the lent world: a
    /// probe-placed point of the room's box on the room seed. `None`: the
    /// world cannot run it; `Some(None)`: no point after the 20 tries.
    fn spawn_point(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: RoomId,
        class: i32,
    ) -> Option<Option<(i32, i32)>> {
        let _ = (sim, h, room, class);
        None
    }
    /// Creates a monster near a unit, around the unit's position in its room,
    /// on the lent world. `None`: the world cannot run it; `Some(None)`:
    /// nothing placed.
    #[allow(clippy::too_many_arguments)]
    fn spawn_near(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        class: i32,
        mode: u8,
        spread: i32,
        flags: u16,
    ) -> Option<Option<UnitId>> {
        let _ = (sim, h, unit, class, mode, spread, flags);
        None
    }
    /// The preset spawn at (x, y) in `room` on the lent world. A class past
    /// the monstats rows is superunique `class - rows`, with its init,
    /// minions and quest links. `None`: the world cannot run it;
    /// `Some(None)`: nothing made.
    #[allow(clippy::too_many_arguments)]
    fn spawn_preset(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: RoomId,
        x: i32,
        y: i32,
        class: i32,
        mode: u8,
    ) -> Option<Option<UnitId>> {
        let _ = (sim, h, room, x, y, class, mode);
        None
    }
    /// The random boss of `class` in `room` on the lent world: the boss with
    /// its modifiers and minions. `None`: the world cannot run it;
    /// `Some(None)`: nothing made.
    fn spawn_random_boss(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: RoomId,
        class: i32,
    ) -> Option<Option<UnitId>> {
        let _ = (sim, h, room, class);
        None
    }
    /// Populates `room` outside the tick's population step, from inside an
    /// action step (the portal far end): ambient spawns, then on the room's
    /// first population its presets, objects and monsters. `false`: not run
    /// (no population here).
    fn populate_off_tick(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        room: RoomId,
    ) -> bool {
        let _ = (sim, h, room);
        false
    }
    /// The umod init of a restored record: the init functions of the fixed
    /// umods 1-4, then of the unit's own list, on `unit` (unique) and its
    /// minions, or on the minion `unit` alone (`unique` false). `false`: not
    /// run (no world here).
    fn restore_umod_init(
        &mut self,
        sim: &mut Sim<'_>,
        h: &mut ActionHooks<X>,
        unit: UnitId,
        unique: bool,
    ) -> bool {
        let _ = (sim, h, unit, unique);
        false
    }
    /// The concrete state back (the lender downcasts it).
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

impl<X> ActionHooks<X> {
    /// Runs `f` on the lent monster world with the hooks; `None` when no
    /// world is lent. A call while the world is already out (a monster
    /// route inside a monster route) is logged as
    /// [`WiringError::Reentrant`] and not run.
    pub fn with_monster_world<R>(
        &mut self,
        f: impl FnOnce(&mut dyn MonsterWorld<X>, &mut Self) -> R,
    ) -> Option<R> {
        let Some(mut w) = self.monster_world.take() else {
            if self.monster_world_out {
                self.errors.push(WiringError::Reentrant("monster world"));
            }
            return None;
        };
        self.monster_world_out = true;
        let r = f(&mut *w, self);
        self.monster_world_out = false;
        self.monster_world = Some(w);
        Some(r)
    }

    /// Runs `f` while a monster route holds the world and runs a call
    /// that is itself a world call with the world in hand (population's
    /// creation inside [`MonsterWorld::spawn_at`]): its allocations take
    /// their type init from that call, as population's own do, so the
    /// hooks read as "no world lent" rather than "out".
    pub fn as_world_holder<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let out = std::mem::replace(&mut self.monster_world_out, false);
        let r = f(self);
        self.monster_world_out = out;
        r
    }

    /// Lends `w` to the hooks from inside a monster route (a umod
    /// callback's mode set): the world the route holds goes back in for the
    /// nested call. Returns the "out"
    /// state to hand to [`Self::take_relent_monster_world`].
    pub fn relend_monster_world(&mut self, w: Box<dyn MonsterWorld<X>>) -> bool {
        let out = self.monster_world_out;
        self.monster_world = Some(w);
        self.monster_world_out = false;
        out
    }

    /// Takes back the world lent by [`Self::relend_monster_world`] and
    /// restores the "out" state; `None` when the nested call did not
    /// return it.
    pub fn take_relent_monster_world<W: 'static>(&mut self, out: bool) -> Option<W> {
        self.monster_world_out = out;
        self.monster_world
            .take()
            .and_then(|w| w.into_any().downcast::<W>().ok())
            .map(|w| *w)
    }

    /// The monster data of `unit` in the lent world.
    pub fn monster_data(&self, unit: UnitId) -> Option<&MonsterData> {
        self.monster_world.as_ref()?.monster(unit)
    }

    /// The monster data of `unit` in the lent world, mutable.
    pub fn monster_data_mut(&mut self, unit: UnitId) -> Option<&mut MonsterData> {
        self.monster_world.as_mut()?.monster_mut(unit)
    }

    /// Runs the umod dispatcher in `mode` on `unit` when a world is lent;
    /// false when none is (the caller then takes its pending default).
    pub fn run_umods(
        &mut self,
        sim: &mut Sim<'_>,
        unit: UnitId,
        arg: Option<UnitId>,
        mode: u8,
    ) -> bool {
        let ran = self
            .with_monster_world(|w, h| w.umods(sim, h, unit, arg, mode))
            .is_some();
        // A umod callback that deals damage (fire's death burst) reaches
        // the attacker's mode-3 dispatch inside the damage-apply step,
        // nested, right after that hit's crit draw: the umod host lends the
        // world back around that hit, so `run_umods` runs it in place. A
        // damage-apply step reached with the world out and not lent back is
        // a wiring error ([`Self::defer_umod_hit`]); its dispatch runs here,
        // after the outer one.
        if ran {
            while !self.deferred_umod_hits.is_empty() {
                let a = self.deferred_umod_hits.remove(0);
                self.with_monster_world(|w, h| w.umods(sim, h, a, None, umod_mode::HIT));
            }
        }
        ran
    }
    /// A mode-3 dispatch for `attacker` asked while the world is out and
    /// not lent back (already logged by [`Self::with_monster_world`]): run
    /// after the outer dispatch.
    pub(super) fn defer_umod_hit(&mut self, attacker: UnitId) -> bool {
        if self.monster_world_out {
            self.deferred_umod_hits.push(attacker);
            true
        } else {
            false
        }
    }
}

impl<X: Pending> ActionHooks<X> {
    /// [`MonsterWorld::restore_umod_init`] on the lent monster world;
    /// false when none is lent or it does not run it.
    pub fn restore_umod_init(&mut self, sim: &mut Sim<'_>, unit: UnitId, unique: bool) -> bool {
        self.with_monster_world(|w, h| w.restore_umod_init(sim, h, unit, unique))
            .unwrap_or(false)
    }

    /// [`MonsterWorld::assign_umod_arg`] on the lent monster world (none:
    /// nothing).
    pub fn assign_umod_arg(&mut self, sim: &mut Sim<'_>, unit: UnitId, umod: u8, unique: bool) {
        self.with_monster_world(|w, h| w.assign_umod_arg(sim, h, unit, umod, unique));
    }

    /// Class reinit on the lent monster world (none: nothing).
    pub fn reinit_class(&mut self, sim: &mut Sim<'_>, unit: UnitId, class: i32, mode: u32) {
        self.with_monster_world(|w, h| w.reinit(sim, h, unit, class, mode));
    }

    /// Sets the monster data's `dwAiState`; a unit without monster data in
    /// the lent world asks [`Pending::set_monster_ai_state`].
    pub fn set_monster_ai_state(&mut self, unit: UnitId, v: u32) {
        match self.monster_data_mut(unit) {
            Some(m) => m.ai_state = v,
            None => self.x.set_monster_ai_state(unit, v),
        }
    }

    /// The AI-state half of the monster mode set: `m` is the mode being
    /// left; nothing for mode 1; old state >= 16 becomes state - 16; state 13
    /// leaving mode 3 stays; else the state is `m`.
    pub fn leave_monster_mode(&mut self, unit: UnitId, m: u32) {
        if m == 1 {
            return;
        }
        let s = self.ai_state_of(unit);
        let new = if s >= 16 {
            s - 16
        } else if s == 13 && m == 3 {
            13
        } else {
            m
        };
        self.set_monster_ai_state(unit, new);
    }

    fn ai_state_of(&self, unit: UnitId) -> u32 {
        match self.monster_data(unit) {
            Some(d) => d.ai_state,
            None => self.x.ai_state(unit),
        }
    }

    /// Whether the monster data's type flags & `mask` is non-zero; a unit
    /// without monster data asks [`Pending::monster_flag`].
    pub fn monster_flag(&self, unit: UnitId, mask: u32) -> bool {
        match self.monster_data(unit) {
            Some(m) => u32::from(m.type_flags) & mask != 0,
            None => self.x.monster_flag(unit, mask),
        }
    }

    /// A monster whose data has type flag 0x02 (superunique) gives its
    /// hcIdx, any other unit none; a unit without monster data asks
    /// [`Pending::superunique`].
    pub fn superunique(&self, unit: UnitId) -> Option<u16> {
        use crate::monsters::init::type_flag;
        match self.monster_data(unit) {
            Some(m) if m.type_flags & type_flag::SUPERUNIQUE != 0 => Some(m.boss_hc_idx),
            Some(_) => None,
            None => self.x.superunique(unit),
        }
    }

    /// The monstats row of a monster with monster data in the lent world.
    fn monstats_of(&self, unit: UnitId) -> Option<&d2_data::tables::Monstats> {
        let class = self.monster_data(unit)?.class;
        self.tables
            .combat
            .monstats
            .get(usize::try_from(class).ok()?)
    }

    /// Whether the unit is a monster whose monstats row has the `boss` flag.
    /// A unit without monster data asks [`Pending::is_boss`].
    pub fn is_boss(&self, unit: UnitId) -> bool {
        match self.monstats_of(unit) {
            Some(m) => m.boss,
            None => self.x.is_boss(unit),
        }
    }

    /// Whether the monstats row has `demon`.
    pub fn is_demon(&self, unit: UnitId) -> bool {
        match self.monstats_of(unit) {
            Some(m) => m.demon,
            None => self.x.is_demon(unit),
        }
    }

    /// Whether the monstats row has `lUndead` or `hUndead`.
    pub fn is_undead(&self, unit: UnitId) -> bool {
        match self.monstats_of(unit) {
            Some(m) => m.lundead || m.hundead,
            None => self.x.is_undead(unit),
        }
    }

    /// Whether the monstats row has `primeevil`.
    pub fn is_prime_evil(&self, unit: UnitId) -> bool {
        match self.monstats_of(unit) {
            Some(m) => m.primeevil,
            None => self.x.is_prime_evil(unit),
        }
    }
}
