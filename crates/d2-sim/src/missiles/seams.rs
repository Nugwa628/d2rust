//! What missile code needs from other systems. Each trait
//! names its expected provider; tests use small fakes. Nothing here
//! decides missile behaviour: the rules stay in the missile modules.

use crate::game::Game;
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};

use super::hit::Damage;
use super::MissileData;

/// Units, stats and states. Provider: the units/stats session.
pub trait MissileUnits {
    /// The rules the game runs under: a mod game's server-do and
    /// server-hit bodies come from them. Default: D2RUST's own.
    fn rules(&self) -> &crate::rules::Ruleset {
        &crate::rules::ORIGINAL
    }
    /// Unit allocation for a missile: one game-seed step
    /// derives the unit seed, the type-3 GUID counter
    /// advances, the unit enters `room` at (x, y) with `mode`. Missile
    /// init ([`super::init_missile`]) runs right after. `None` = failure.
    fn alloc_missile(
        &mut self,
        game: &mut Game,
        class: u16,
        x: i32,
        y: i32,
        room: RoomId,
        mode: u8,
    ) -> Option<UnitId>;
    /// Unit removal (frees the unit, cancels its timers).
    fn remove_unit(&mut self, game: &mut Game, unit: UnitId);
    /// The unit seed.
    fn seed(&mut self, unit: UnitId) -> &mut Seed;
    /// Position in subtiles.
    fn position(&self, unit: UnitId) -> (i32, i32);
    /// Unit size.
    fn size(&self, unit: UnitId) -> i32;
    /// Stat value (layer 0).
    fn stat(&self, unit: UnitId, stat: u16) -> i32;
    /// Base stat value.
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32;
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32);
    fn has_state(&self, unit: UnitId, state: u16) -> bool;
    /// `stat` of the stat list of `state` on the unit; `None` without one.
    fn state_stat(&self, unit: UnitId, state: u16, stat: u16) -> Option<i32>;
    /// Allocates the missile's stat list.
    fn alloc_stat_list(&mut self, unit: UnitId);
    /// Unit flag test / set.
    fn unit_flag(&self, unit: UnitId, bit: u32) -> bool;
    fn set_unit_flag(&mut self, unit: UnitId, bit: u32, on: bool);
    /// Whether the attacker may attack the defender.
    fn may_attack(&self, attacker: UnitId, defender: UnitId) -> bool;
    /// Alignment: 0 evil, 1 neutral, 2 good.
    fn alignment(&self, unit: UnitId) -> u8;
    /// Monster that is a hireling.
    fn is_hireling(&self, unit: UnitId) -> bool;
    /// `justhit`: a stat list with state 86 expiring
    /// at `expire` on the unit, state 86 on, and a type-12 timer at
    /// `expire`.
    fn apply_justhit(&mut self, game: &mut Game, unit: UnitId, expire: i32);
}

/// Path primitives. Provider: the units code (path
/// movement).
pub trait MissilePath {
    /// The unit has a path.
    fn has_path(&self, unit: UnitId) -> bool;
    fn set_velocity(&mut self, unit: UnitId, v: i32);
    fn velocity(&self, unit: UnitId) -> i32;
    fn set_target_unit(&mut self, unit: UnitId, target: UnitId);
    fn set_target_point(&mut self, unit: UnitId, x: i32, y: i32);
    /// Footprint mask.
    fn set_footprint_mask(&mut self, unit: UnitId, mask: u16);
    /// Move-test mask.
    fn set_move_mask(&mut self, unit: UnitId, mask: u16);
    /// Build the path toward the target.
    fn build(&mut self, game: &mut Game, unit: UnitId);
    /// Path acceleration and maximum velocity.
    fn set_acceleration(&mut self, unit: UnitId, accel: i32, max_velocity: i32);
    /// Distance to the target point.
    fn target_distance(&self, unit: UnitId) -> i32;
    /// Unit step: false when it returns 2 (no movement).
    fn step(&mut self, game: &mut Game, unit: UnitId) -> bool;
    /// Collision word under the unit, all bits.
    fn collision_word(&self, game: &Game, unit: UnitId) -> u16;
    /// Subtiles crossed by the last step, in path order.
    fn crossed_subtiles(&self, unit: UnitId) -> Vec<(i32, i32)>;
}

/// Rooms and collision maps. Provider: the DRLG session.
pub trait MissileRooms {
    /// The room containing (x, y), searched from `near` and
    /// its adjacent rooms.
    fn find_room(&self, game: &Game, near: RoomId, x: i32, y: i32) -> Option<RoomId>;
    /// The room is in town.
    fn in_town(&self, game: &Game, room: RoomId) -> bool;
    /// Collision at (x, y) with `size`, masked by `mask`.
    fn collision_mask(
        &self,
        game: &Game,
        room: RoomId,
        x: i32,
        y: i32,
        size: i32,
        mask: u16,
    ) -> u16;
    /// Collision at the subtile, masked by `mask`.
    fn collision_at(&self, game: &Game, room: RoomId, x: i32, y: i32, mask: u16) -> u16;
    /// Clear collision bit 0x40 under the unit with its size.
    fn clear_footprint(&mut self, game: &mut Game, unit: UnitId);
    /// The units hit by a query of size `r` (the missile's size) at
    /// subtile (x, y), in the provider's search order. Flight selects the first
    /// unit accepted by the collide callback.
    fn units_at(&self, game: &Game, room: RoomId, x: i32, y: i32, r: i32) -> Vec<UnitId>;
}

/// Damage and skill code. Provider: the combat/skills session.
pub trait MissileCombat {
    /// Damage setup,
    /// with its draws. `skill` is the missile's stored skill. Returns the missile data flag bits it sets.
    fn damage_setup(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        origin: Option<UnitId>,
        missile: UnitId,
        skill: i32,
        level: i32,
    ) -> u32;
    /// To-hit: one draw
    /// on the owner's seed. Only called with an owner (a missing owner
    /// misses without a draw, R5 step 5).
    fn hit_test(&mut self, game: &mut Game, owner: UnitId, defender: UnitId, tohit: i32) -> bool;
    /// The rest of the damage application after the missile's result flags: block /
    /// dodge, hit class, hit flags, pierce percent, events and damage
    /// execution. `data` is the missile's data (class, data
    /// flags), passed because the store is lent out during the hit.
    /// `store` is the hit's missile store: a kill inside the damage apply
    /// runs the victim's mode set, whose umod callbacks create missiles
    /// nested in the hit (umod 23's cross burst), so a hook that makes
    /// missiles borrows it for the call.
    #[allow(clippy::too_many_arguments)]
    fn apply_damage(
        &mut self,
        game: &mut Game,
        store: &mut super::MissileStore,
        owner: Option<UnitId>,
        missile: UnitId,
        unit: UnitId,
        damage: &mut Damage,
        data: Option<&MissileData>,
    );
    /// Unit event 0 (hit by missile), also with no unit.
    fn hit_by_missile_event(&mut self, game: &mut Game, missile: UnitId, unit: Option<UnitId>);
    /// Damage-percent bonuses of the hit unit's kind: stat 121 for
    /// demons, 122 for undead, 180 by monster type, summed.
    fn target_damage_bonus(&self, missile: UnitId, unit: UnitId) -> i32;
    /// Armor += `delta` (missile stat 120), clamped at ≥ 0.
    fn add_target_ac(&mut self, game: &mut Game, unit: UnitId, delta: i32);
}

/// Hooks of skill and monster code run by creation. Providers: the
/// combat and skills code (init callbacks), monster init (unique
/// mods).
pub trait MissileHooks {
    /// The parameter record's init callback.
    fn init_callback(&mut self, game: &mut Game, missile: UnitId, callback: u32, arg: u32);
    /// Unique-mod hook for a monster owner.
    ///
    /// `store` is the creation's missile store: a hook that makes or reads
    /// missiles (the multishot copies) borrows it for the
    /// call.
    fn unique_mod_missile(
        &mut self,
        game: &mut Game,
        store: &mut super::MissileStore,
        owner: UnitId,
        missile: UnitId,
    );
}

/// The skills formula a server body evaluates
/// (`eval(owner, k.<field>, k, level)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillCalc {
    Calc1,
    Calc2,
    AuraRange,
    AuraLen,
    Calc4,
}

/// A raw skills column a body reads (offsets by
/// the field table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillField {
    /// `Param1`…`Param8`; the index is 1…8.
    Param(u8),
    /// `aurafilter`.
    AuraFilter,
    /// `auratargetstate`.
    AuraTargetState,
    /// `pettype`.
    PetType,
}

/// What `summon_class` answers:
/// the class (< 0: none) and the mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SummonClass {
    pub class: i32,
    pub mode: i32,
}

/// What the server-do and server-hit bodies need beyond
/// the other seams. Providers: skills (formulas, the area scan of
/// the skills area scan), path (new-step flag, target), DRLG
/// (collision writes), units (animation frame, alive), combat (the area
/// hit). Every default is the narrowest reading: nothing happens, or the
/// value that makes the caller do nothing.
#[allow(unused_variables)]
pub trait MissileBodies {
    /// A missiles
    /// formula.
    fn missile_calc(
        &mut self,
        game: &mut Game,
        missile: UnitId,
        owner: Option<UnitId>,
        field: u32,
        class: i32,
        level: i32,
    ) -> i32 {
        0
    }
    /// The skills record of `skill` exists.
    fn skill_exists(&self, skill: i32) -> bool {
        false
    }
    /// `eval(owner, k.<calc>, k, level)` (the
    /// owner may be none).
    fn skill_calc(
        &mut self,
        game: &mut Game,
        owner: Option<UnitId>,
        skill: i32,
        calc: SkillCalc,
        level: i32,
    ) -> i32 {
        0
    }
    /// The path's new-step flag.
    fn path_new_step(&self, unit: UnitId) -> bool {
        false
    }
    /// The unit's path target, after the path-target refresh.
    fn path_target(&mut self, game: &Game, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// OR `bits` into the one collision
    /// cell at (x, y) of the room containing it, searched from `room`.
    fn or_collision(&mut self, game: &mut Game, room: RoomId, x: i32, y: i32, bits: u16) {}
    /// Stamp `bits` with the unit's size at its position.
    fn stamp_collision(&mut self, game: &mut Game, unit: UnitId, bits: u16) {}
    /// The unit's animation frame (8.8).
    fn anim_frame(&self, unit: UnitId) -> i32 {
        0
    }
    fn set_anim_frame(&mut self, unit: UnitId, v: i32) {}
    /// The unit is dead.
    fn is_dead(&self, unit: UnitId) -> bool {
        false
    }
    /// Demon.
    fn is_demon(&self, unit: UnitId) -> bool {
        false
    }
    /// Undead.
    fn is_undead(&self, unit: UnitId) -> bool {
        false
    }
    /// The units `scan_unit(game, owner, x, y, r, f, …, noaura 0)`
    /// (the skills area scan) hands its callback, in order.
    fn area_units(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        at: (i32, i32),
        r: i32,
        f: u32,
    ) -> Vec<UnitId> {
        Vec::new()
    }
    /// The area damage `(game, owner, U, record)` on a copy of the record.
    fn area_hit(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        unit: UnitId,
        record: &crate::combat::DamageRecord,
    ) {
    }

    // ---- More server bodies --------------

    /// A raw skills column of `skill` (0 without a record).
    fn skill_field(&self, skill: i32, field: SkillField) -> i32 {
        0
    }
    /// `scan_unit(game, owner, x, y, r, f, …, noaura)` (the skills area
    /// scan) as [`Self::area_units`], with the `noaura` argument (the
    /// default has no scan for `noaura` 1).
    fn scan_units(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        at: (i32, i32),
        r: i32,
        f: u32,
        noaura: bool,
    ) -> Vec<UnitId> {
        if noaura {
            Vec::new()
        } else {
            self.area_units(game, owner, at, r, f)
        }
    }
    /// The pet type count.
    fn pet_type_count(&self) -> i32 {
        0
    }
    /// The states count.
    fn states_count(&self) -> i32 {
        0
    }
    /// The overlay count.
    fn overlay_count(&self) -> i32 {
        0
    }
    /// `phys_min(O, k, L, 1)` then `phys_max(O, k, L, 1)`,
    /// in that order.
    fn skill_phys(
        &mut self,
        game: &mut Game,
        owner: Option<UnitId>,
        skill: i32,
        level: i32,
    ) -> (i32, i32) {
        (0, 0)
    }
    /// `elem_len(O, k, L, 1)`.
    fn skill_elem_len(&mut self, game: &mut Game, owner: UnitId, skill: i32, level: i32) -> i32 {
        0
    }
    /// The param 1 of `owner`'s skill entry for `skill`;
    /// `None` without an entry.
    fn skill_entry_param1(&self, owner: UnitId, skill: i32) -> Option<i32> {
        None
    }
    /// Max life.
    fn max_life(&self, unit: UnitId) -> i32 {
        0
    }
    /// Max mana.
    fn max_mana(&self, unit: UnitId) -> i32 {
        0
    }
    fn overlay(&mut self, game: &mut Game, unit: UnitId, overlay: i32) {}
    /// The monster's class has the monstats2 `large` flag.
    fn is_large_monster(&self, unit: UnitId) -> bool {
        false
    }
    /// The monster's class has the monstats2 `small` flag
    /// (moltenboulder, server-damage function 14).
    fn is_small_monster(&self, unit: UnitId) -> bool {
        false
    }
    /// Pet test.
    fn is_pet(&self, game: &Game, owner: UnitId, unit: UnitId, skill: i32) -> bool {
        false
    }
    /// Ally test.
    fn is_ally(&self, game: &Game, owner: UnitId, unit: UnitId, skill: i32) -> bool {
        false
    }
    /// Ally test `(game, owner, unit)` (the skills ally rule
    /// flag 0x10000).
    fn ally_test(&mut self, game: &mut Game, owner: UnitId, unit: UnitId) -> bool {
        false
    }
    /// `accepts(source, unit, f)` (the skills unit
    /// filter).
    fn accepts(&self, game: &Game, source: UnitId, unit: UnitId, f: u32) -> bool {
        false
    }
    /// The unit's target position (the skills target rule:
    /// the target unit's position, else the path target point).
    fn target_position(&mut self, game: &Game, unit: UnitId) -> Option<(i32, i32)> {
        None
    }
    /// The path target point.
    fn path_target_point(&self, unit: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// Path type.
    fn set_path_type(&mut self, unit: UnitId, ty: i32) {}
    /// Path distance.
    fn set_path_distance(&mut self, unit: UnitId, d: i32) {}
    /// The 64-step direction from the unit's
    /// position to (x, y) (the path direction rule).
    /// Default: 0.
    fn dir64(&self, unit: UnitId, at: (i32, i32)) -> i32 {
        0
    }
    /// Path point i := (x, y) (the path's point array).
    fn set_path_point(&mut self, unit: UnitId, i: i32, at: (u16, u16)) {}
    /// The path's point count := n.
    fn set_path_point_count(&mut self, unit: UnitId, n: i32) {}
    fn path_teleport(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        room: Option<RoomId>,
        x: i32,
        y: i32,
    ) {
    }
    /// Refresh the room.
    fn refresh_room(&mut self, game: &mut Game, room: RoomId) {}
    /// The line test reports a hit.
    fn line_hits(
        &self,
        game: &Game,
        room: RoomId,
        from: (i32, i32),
        to: (i32, i32),
        mask: u16,
    ) -> bool {
        false
    }
    /// The room seed; `None` without one.
    fn room_seed(&mut self, game: &mut Game, room: RoomId) -> Option<&mut Seed> {
        None
    }
    /// Skill server-do `index` with `caster` as the caster
    /// (index ≤ 190 unsigned and a non-null
    /// entry, else nothing); its result is ignored. The store is the
    /// running one: the body's caster is a missile in it.
    fn skill_srv_do(
        &mut self,
        game: &mut Game,
        store: &mut super::MissileStore,
        caster: UnitId,
        index: i32,
        skill: i32,
        level: i32,
    ) {
    }
    /// `shout_state(game, unit, owner, skill, level)` (the
    /// shout state rule).
    fn shout_state(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        owner: UnitId,
        skill: i32,
        level: i32,
    ) {
    }
    /// The unit's stat list of state `s`; its expiry frame
    /// when there is one.
    fn state_list_expiry(&self, unit: UnitId, s: i32) -> Option<i32> {
        None
    }
    /// A fresh list (game pool, flags 2, expiry `expire`,
    /// `owner`'s type and GUID), state id `s`, remove callback
    /// Attached to the unit, state `s` on. False when the
    /// allocation fails.
    fn new_state_list(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        s: i32,
        expire: i32,
        owner: UnitId,
    ) -> bool {
        false
    }
    /// `aura_fill(unit, list of s, k record, k, L)` on the unit's
    /// existing list of `s`.
    fn aura_fill(&mut self, game: &mut Game, unit: UnitId, s: i32, skill: i32, level: i32) {}
    /// Mark state `s` changed.
    fn mark_state_changed(&mut self, unit: UnitId, s: i32) {}
    /// The expiry of the unit's list of `s` (nothing
    /// without one).
    fn set_state_list_expiry(&mut self, unit: UnitId, s: i32, expire: i32) {}
    /// `apply_state` with source
    /// `owner`, target `unit`, skill, level, `duration`, stat field 0
    /// value 0, state `s`, callback 0. True when it made a list.
    #[allow(clippy::too_many_arguments)]
    fn apply_state(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        unit: UnitId,
        skill: i32,
        level: i32,
        duration: i32,
        s: i32,
    ) -> bool {
        false
    }
    /// Terror install.
    #[allow(clippy::too_many_arguments)]
    fn terror(
        &mut self,
        game: &mut Game,
        source: UnitId,
        unit: UnitId,
        skill: i32,
        a: i32,
        b: i32,
    ) {
    }
    /// `summon_class(O, k, L, &mode)` (the skills
    /// summon rule).
    fn summon_class(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        skill: i32,
        level: i32,
    ) -> SummonClass {
        SummonClass { class: -1, mode: 0 }
    }
    /// `summon_spawn` with flags
    /// 0xD, AI special state 0, pet max 0.
    #[allow(clippy::too_many_arguments)]
    fn summon_spawn(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        class: i32,
        mode: i32,
        at: (i32, i32),
        pet_type: i32,
    ) -> Option<UnitId> {
        None
    }
    /// Bone Wall step 8 on piece `piece` of anchor
    /// `anchor`, in order: owner data (anchor GUID, 1, 0, 0)
    /// (owner data); minion list; umod 15;
    /// `skill_stats(game, owner, piece, k, L, 0)`; stored
    /// owner := `owner`; set the piece's mode `(game, piece, 0, 9)`.
    fn bind_bone_wall_piece(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        anchor: UnitId,
        piece: UnitId,
        skill: i32,
        level: i32,
    ) {
    }
    /// The portal object `(game, owner, room, x, y, level, 0,
    /// class, 1)`.
    #[allow(clippy::too_many_arguments)]
    fn create_portal(
        &mut self,
        game: &mut Game,
        owner: Option<UnitId>,
        room: Option<RoomId>,
        at: (i32, i32),
        level: i32,
        class: i32,
    ) {
    }
    /// Sound event.
    fn unit_sound(&mut self, game: &mut Game, unit: UnitId, id: i32) {}
    /// The chest drop with operate context {game, chest,
    /// operator none, chest seed, chest class} (treasure class
    /// Q = 4).
    fn chest_drop(&mut self, game: &mut Game, chest: UnitId) {}
    /// Floor drop spot `(room, (x, y), &out, size 1, fallback
    /// 1)`.
    fn floor_drop_spot(
        &mut self,
        game: &Game,
        room: RoomId,
        at: (i32, i32),
    ) -> Option<(RoomId, i32, i32)> {
        None
    }
    /// The gold item request of the gold-drop body (step 4.3)
    /// for `missile` at the spot.
    fn create_gold(&mut self, game: &mut Game, missile: UnitId, room: RoomId, at: (i32, i32)) {}
    /// Unit mode of a non-missile unit.
    fn unit_mode(&self, unit: UnitId) -> i32 {
        0
    }
    fn set_unit_mode(&mut self, unit: UnitId, mode: i32) {}
    /// The objects record's `FrameCnt1` of the object's class
    /// `None` without a record.
    fn object_frame_cnt1(&self, unit: UnitId) -> Option<i32> {
        None
    }
    /// The active room's sub-tile rectangle (x, y, w, h) read by the
    /// unit find;
    /// `None` without one.
    fn room_subtiles(&self, game: &Game, room: RoomId) -> Option<(i32, i32, i32, i32)> {
        None
    }
    /// (Redemption's
    /// per-corpse effect; open question).
    #[allow(clippy::too_many_arguments)]
    fn redemption_effect(
        &mut self,
        game: &mut Game,
        missile: UnitId,
        unit: UnitId,
        skill: i32,
        level: i32,
        last: bool,
    ) {
    }
    /// The `mon` list of the levels record of the room's level.
    fn level_mon_list(&self, game: &Game, room: RoomId) -> Vec<i32> {
        Vec::new()
    }
    /// The monstats `isSpawn` flag of `class`; `None` for an invalid class
    /// (a read through a null record).
    fn monster_is_spawn(&self, class: i32) -> Option<bool> {
        None
    }
    /// The monster create request {room, class,
    /// mode 1, GUID 0, x, y, spread 5, flags 0}; true when
    /// created.
    fn create_monster(
        &mut self,
        game: &mut Game,
        room: RoomId,
        class: i32,
        at: (i32, i32),
    ) -> bool {
        false
    }
    /// The monstats count.
    fn monstats_count(&self) -> i32 {
        0
    }
    fn spawn_monster(
        &mut self,
        game: &mut Game,
        room: Option<RoomId>,
        class: i32,
        at: (i32, i32),
        mode: i32,
    ) {
    }
    /// The rabies poison.
    #[allow(clippy::too_many_arguments)]
    fn rabies_poison(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        unit: UnitId,
        t: i32,
        skill: i32,
        level: i32,
    ) {
    }
    /// The quest test `(game, id)` (no entry with `id` → true;
    /// else its byte +9 = 1).
    fn quest_test(&self, game: &Game, id: i32) -> bool {
        false
    }
    /// (Tyrael's spawn; open
    /// question).
    fn spawn_tyrael(&mut self, game: &mut Game, room: Option<RoomId>, missile: UnitId) {}
}

/// Everything missile code needs.
pub trait MissileWorld:
    MissileUnits
    + MissilePath
    + MissileRooms
    + MissileCombat
    + MissileHooks
    + MissileBodies
    + AsDynMissile
{
}

impl<
        T: MissileUnits
            + MissilePath
            + MissileRooms
            + MissileCombat
            + MissileHooks
            + MissileBodies
            + AsDynMissile
            + ?Sized,
    > MissileWorld for T
{
}

/// A world as `dyn` [`MissileWorld`], so generic missile code can hand
/// it to a mod game's bodies ([`super::provider::MissileProvider`]).
pub trait AsDynMissile {
    fn as_dyn_missile(&mut self) -> &mut dyn MissileWorld;
}

impl<
        T: MissileUnits + MissilePath + MissileRooms + MissileCombat + MissileHooks + MissileBodies,
    > AsDynMissile for T
{
    fn as_dyn_missile(&mut self) -> &mut dyn MissileWorld {
        self
    }
}
