//! The seams of the action wiring that have no provider of their own: path
//! and position, monster data, items, skill use, objects, sounds, messages
//! and the AI target sources.
//!
//! The adapters of [`super`] call these where the real provider is
//! missing. Every default is the narrowest reading: nothing happens, or
//! the value that makes the caller do nothing. A host (or a test)
//! overrides what it can provide; [`NoPending`] takes every default.
//! When a provider lands, its methods move out of this trait into the
//! adapter that wires it.

use crate::game::Game;
use crate::monsters::ai::{ModeTarget, PortalNpc, QuestCall};
use crate::units::hooks::Sim;
use crate::units::{RoomId, UnitId, UnitType};

use super::ActionHooks;

/// A skill timer event the unit dispatch hands to the skills, with its
/// arguments as the dispatch passes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillEvent {
    /// Event 5: `srvactivefunc` `f` of the skill's aura state (already
    /// checked < 191), the skill (a1) and a2.
    ActiveState {
        unit: UnitId,
        f: u16,
        skill: u32,
        arg2: u32,
    },
    /// Event 8 with its two arguments.
    Periodic { unit: UnitId, arg1: u32, arg2: u32 },
    /// Event 9 after its checks: a1, the skill (a2) and the level `l` =
    /// total(151, layer = skill).
    ItemAura {
        unit: UnitId,
        arg1: u32,
        skill: u32,
        level: i32,
    },
    /// The remove callback `cb` of a list the expiry walk freed whose body
    /// is a skill body: Confuse, Attract, Conversion or Mind Blast (called
    /// with the unit and the state).
    ListRemoved {
        unit: UnitId,
        state: u32,
        cb: u32,
        list: crate::stats::lists::ListId,
    },
}

/// A step of the kill whose callee has no body in d2-sim, in the kill's
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KillStep {
    /// Pet death bookkeeping (first step).
    PetCredit,
    /// The arena kill event (second step).
    AttackerBookkeeping,
    /// The death mode faces the attacker (path direction).
    FaceAttacker,
    /// Quest kill parse (not run for a revived monster).
    QuestKill,
    /// The act 5 barricade doors (`objCol` monsters open object 571 /
    /// 572 at the same spot).
    BarricadeDoors,
}

/// What a host's seams queue for the quest control (the kill parse
/// and the quest add link); the quest-owning host drains them each
/// tick ([`Pending::take_quest_events`]). D2RUST's own: the play
/// host's route from the monster seams to the quest control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestEvent {
    /// Monster init linked `unit` to quest chain `chain`.
    Link { unit: UnitId, chain: u8 },
    /// `victim` died, `killer` the attacker.
    Kill {
        victim: UnitId,
        killer: Option<UnitId>,
    },
    /// Radament's AI ran in his level.
    RadamentActivated { unit: UnitId },
    /// The Summoner's AI first ran.
    SummonerActivated,
    /// The cube placed a Horadric Staff for `player`.
    StaffAssembled { player: UnitId },
    /// Shenk's AI ran.
    ShenkActivated { unit: UnitId },
    /// Nihlathak's AI ran.
    NihlathakActivated,
    /// An Ancient's AI armed-off call.
    AncientsDisarm,
    /// The Baal crab reached the Worldstone portal.
    BaalToStairs,
    /// Anya's AI asks for the temple portal.
    AnyaOpenPortal { unit: UnitId },
    /// The baalfx control missile asks for Tyrael (from server-do 36 /
    /// server-hit 57), with the missile's room and position at the call.
    SpawnTyrael {
        room: Option<RoomId>,
        missile: UnitId,
        x: i32,
        y: i32,
    },
    /// A caged barbarian left through its group's portal (from the prisoner
    /// AI), by GUID (the unit is removed in the same call).
    WussieLeft { guid: u32 },
    /// The prisoner AI's rescue of `unit` by `player`.
    WussieRescue { player: UnitId, unit: UnitId },
    /// The prisoner AI's wait hook.
    WussieWait,
    /// A C->S 0x44 message: `player` puts `item` (GUID, 0: none) into the
    /// object with GUID `object` (`action` 2 cancel, 3 insert).
    InsertItem {
        player: UnitId,
        object: u32,
        item: u32,
        action: u16,
    },
    /// A special monster was created (the Golden Bird's boss choice).
    BossCreated { unit: UnitId },
    /// A preset superunique's quest hook (the Travincal council).
    PresetBoss { unit: UnitId },
    /// Alkor's map AI cleared the Golden Bird quest state.
    AlkorReset,
    /// Ormus' map AI activates the Gidbinn altar.
    OrmusAltar,
}

/// Host-only mark `[MARK, child GUID LE u32]` addressed to the parent
/// item, left by a host whose [`Pending::socket_restore`] cannot reach its
/// inventory model inside the call: the host links the child into the
/// parent at this place of the tick.
pub const SOCKET_RESTORE_MARK: u8 = 0xF6;

/// Host-only mark `[MARK, 0, 0, 0, 0]` addressed to a player whose key test
/// ([`Pending::object_key_test`]) passed: a locked chest or door took a key,
/// and the host uses one up on its inventory model at this place of the
/// tick (`items::moves::handlers::use_key`).
pub const KEY_USE_MARK: u8 = 0xF4;

/// Seams without a provider (see the module doc). Grouped by the part of the
/// game that will own them.
#[allow(unused_variables)]
pub trait Pending {
    /// How many messages the host holds unsent, `None` when it cannot say
    /// (see `ActionHooks::pet_follow_mark`).
    fn sent_len(&self) -> Option<usize> {
        None
    }

    // ---- animation ------------------------------------------------

    /// The COF name built for a unit in a mode (token + mode + weapon class,
    /// NUL-padded to 8 bytes) for the AnimData lookup. The composer for
    /// players, objects and units with an inventory is still open. `None`:
    /// no name (no record).
    fn anim_name(&self, unit: UnitId, ty: UnitType, class: u32, mode: u32) -> Option<[u8; 8]> {
        None
    }
    /// The animation rate from the AnimData speed (`None` when the unit has
    /// no record) and the rate stats and states.
    fn anim_rate(&self, unit: UnitId, speed: Option<u32>) -> i16 {
        0
    }
    /// The type class of an item (its weapon class `wclass`, else 0), used
    /// for the frame bonus. Default: 0.
    fn item_type_class(&self, item: UnitId) -> u32 {
        0
    }

    // ---- path and position --------------------------------------

    /// Position in subtiles.
    fn position(&self, unit: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// Places a unit at allocation (allocation arguments x, y).
    fn place(&mut self, unit: UnitId, x: i32, y: i32) {}
    /// Unit size.
    fn size(&self, unit: UnitId) -> i32 {
        0
    }
    /// Whether the unit has a path.
    fn has_path(&self, unit: UnitId) -> bool {
        false
    }
    /// Sets the path velocity.
    fn set_velocity(&mut self, unit: UnitId, v: i32) {}
    /// The path velocity.
    fn velocity(&self, unit: UnitId) -> i32 {
        0
    }
    /// Sets the path target unit.
    fn set_target_unit(&mut self, unit: UnitId, target: UnitId) {}
    /// Sets the path target point.
    fn set_target_point(&mut self, unit: UnitId, x: i32, y: i32) {}
    /// Sets the path footprint mask.
    fn set_footprint_mask(&mut self, unit: UnitId, mask: u16) {}
    /// Sets the path move mask.
    fn set_move_mask(&mut self, unit: UnitId, mask: u16) {}
    /// Builds the unit's path.
    fn build_path(&mut self, game: &mut Game, unit: UnitId) {}
    /// Sets the path acceleration and maximum velocity.
    fn set_acceleration(&mut self, unit: UnitId, accel: i32, max_velocity: i32) {}
    /// The distance to the path target.
    fn target_distance(&self, unit: UnitId) -> i32 {
        0
    }
    /// One unit step: false when there is no movement.
    fn step(&mut self, game: &mut Game, unit: UnitId) -> bool {
        false
    }
    /// The collision word cached in the path by the last step. `None`: not
    /// modelled; the adapter then reads the room's grid at the current
    /// position.
    fn cached_collision_word(&self, unit: UnitId) -> Option<u16> {
        None
    }
    /// Subtiles crossed by the last step, in path order.
    fn crossed_subtiles(&self, unit: UnitId) -> Vec<(i32, i32)> {
        Vec::new()
    }
    /// The target point or unit handed to a mode start (AI mode requests).
    fn set_mode_target(&mut self, unit: UnitId, target: ModeTarget) {}
    /// The path step count (AI): the stop distance, without the path
    /// provider. Default: nothing.
    fn set_path_steps(&mut self, unit: UnitId, steps: i32) {}
    /// Whether path flag 0x800 (blocked step) is set.
    fn path_blocked(&self, unit: UnitId) -> bool {
        false
    }
    /// Stops the unit's path without the path provider. Default: nothing.
    fn stop_path(&mut self, unit: UnitId) {}

    // ---- unit queries ---------------------------------------------

    /// Whether `attacker` may attack `defender` (hostility).
    fn may_attack(&self, attacker: UnitId, defender: UnitId) -> bool {
        false
    }
    /// The unit's alignment: 0 evil, 1 neutral, 2 good.
    fn alignment(&self, unit: UnitId) -> u8 {
        0
    }

    // ---- inactive store ([`super::inactive`]): the facts no d2-sim
    // provider holds and the re-creation of stored units.

    /// The room's level has `SaveMonsters` non-zero (leveldefs). Default: no.
    fn save_monsters(&self, game: &Game, room: RoomId) -> bool {
        false
    }
    /// The unit has state `state`. Default: no.
    fn unit_has_state(&self, unit: UnitId, state: u16) -> bool {
        false
    }
    /// The unit has a state with the `udead` flag.
    /// Default: no.
    fn udead_state(&self, unit: UnitId) -> bool {
        false
    }
    /// monstats2 `restore` of the monster; `None`: no row. Default: 1 (the
    /// value of most rows).
    fn monster_restore(&self, unit: UnitId) -> Option<u8> {
        Some(1)
    }
    /// The monster's owner is a player and it passes the pet test. Default:
    /// no.
    fn player_pet(&self, unit: UnitId) -> bool {
        false
    }
    /// Objects: (`Restore` = 0, the object's flag byte has 0x2,
    /// `RestoreVirgins` non-zero). Default: (false, false, false).
    fn object_restore_facts(&self, unit: UnitId) -> (bool, bool, bool) {
        (false, false, false)
    }
    /// One step of the room's seed; `None`: the room has no seed here (the
    /// caller then counts it as a step whose value is not a multiple of 3).
    fn room_seed_step(&mut self, game: &Game, room: RoomId) -> Option<u32> {
        None
    }
    /// The fields of a monster record that need monster data, the AI
    /// control or the owner; the caller fills the rest.
    fn monster_record_extra(&self, unit: UnitId, rec: &mut crate::units::inactive::MonsterRecord) {}
    /// The fields of an "other" record that need object data (the mode-2
    /// switch, event 5 time, GUID and flag byte, and two more object
    /// bytes).
    fn other_record_extra(&mut self, unit: UnitId, rec: &mut crate::units::inactive::OtherRecord) {}
    /// The inactive item record (the save-form bit stream of the item and
    /// its socketed items). Default: none (the item is freed without a
    /// record).
    fn item_record(&mut self, item: UnitId) -> Option<crate::units::inactive::ItemRecord> {
        None
    }
    /// The items in `item`'s own inventory (its socketed children), in
    /// list order, for the inactive item record. Default: none (no
    /// inventory model).
    fn socketed_items(&self, item: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    /// A child of a restored inactive item record put into the parent's
    /// inventory, then the parent refreshed. Default: nothing.
    fn socket_restore(&mut self, game: &mut Game, parent: UnitId, child: UnitId) {}
    /// The path part of detaching a unit from its room: precise and client
    /// x / y set to 0, point count 0, previous room, path flag 0x2.
    fn detach_path(&mut self, unit: UnitId, room: RoomId) {}
    /// Re-creates an item record with ground expiry `expiry` in `room`.
    fn restore_item(
        &mut self,
        game: &mut Game,
        room: RoomId,
        rec: &crate::units::inactive::ItemRecord,
        expiry: i32,
    ) {
    }
    /// Whether the room is in level 108 and passes the sanctuary gate check.
    /// Default: no.
    fn sanctuary_gate(&self, game: &Game, room: RoomId) -> bool {
        false
    }
    /// Whether the monster type flags & `mask` is non-zero (2 superunique,
    /// 4 champion, 8 unique).
    fn monster_flag(&self, unit: UnitId, mask: u32) -> bool {
        false
    }
    /// Whether the unit is a boss.
    fn is_boss(&self, unit: UnitId) -> bool {
        false
    }
    /// Whether the unit is a demon.
    fn is_demon(&self, unit: UnitId) -> bool {
        false
    }
    /// Whether the unit is undead.
    fn is_undead(&self, unit: UnitId) -> bool {
        false
    }
    /// Whether the unit is a prime evil.
    fn is_prime_evil(&self, unit: UnitId) -> bool {
        false
    }
    /// Whether unit flag 0x80000000 (revived) is set.
    fn is_revived(&self, unit: UnitId) -> bool {
        false
    }
    /// Whether the unit is in a moving mode.
    fn moving_mode(&self, unit: UnitId) -> bool {
        false
    }
    /// The unit's melee range.
    fn melee_range(&self, unit: UnitId) -> i32 {
        0
    }
    /// Whether `d` is within melee `range` of `a`.
    fn in_melee_range(&self, a: UnitId, d: UnitId, range: i32) -> bool {
        false
    }
    /// Whether the line between `a` and `b` is blocked (flags 4).
    fn line_blocked(&self, game: &Game, a: UnitId, b: UnitId) -> bool {
        false
    }
    /// Whether `unit` can reach `target` directly.
    fn can_reach_directly(&self, game: &Game, unit: UnitId, target: UnitId) -> bool {
        false
    }
    /// A free teleport spot and its room.
    fn find_spot(&mut self, game: &mut Game, unit: UnitId) -> Option<(i32, i32, RoomId)> {
        None
    }
    /// The room's "LOS draw" test.
    fn los_draw(&self, game: &Game, room: RoomId) -> bool {
        false
    }
    /// Whether the monster class has the mode.
    fn class_has_mode(&self, class: i32, mode: u8) -> bool {
        false
    }
    /// The unit type the unit's mode converts to; `None` = the unit's own
    /// type.
    fn converted_type(&self, unit: UnitId) -> Option<i32> {
        None
    }
    /// Whether the layer's monster type matches `montype`.
    fn montype_matches(&self, layer: u16, montype: i32) -> bool {
        false
    }
    /// The monster bonus for `players` players in the game.
    fn player_count_bonus(&self, players: i32) -> i32 {
        0
    }

    // ---- monster data ---------------------------------------------

    /// Monster level.
    fn monster_level(&self, unit: UnitId) -> i32 {
        0
    }
    /// The monster data's `dwAiState`.
    fn ai_state(&self, unit: UnitId) -> u32 {
        0
    }
    /// The monster data's vision record: the seen value.
    fn vision_seen(&self, unit: UnitId) -> Option<u32> {
        None
    }
    /// Sets the vision record's seen value.
    fn mark_seen(&mut self, unit: UnitId, value: u32) {}
    /// Type-10 handler: resets two AI fields of the monster data to 0.
    fn ai_reset(&mut self, unit: UnitId) {}
    /// Whether a player is interacting with the unit.
    fn interacting(&self, unit: UnitId) -> bool {
        false
    }
    /// The busy test without its interaction part (a cursor item): the
    /// interact info is the unit record's
    /// ([`crate::units::record::InteractInfo`]), which the callers test
    /// first.
    fn busy(&self, unit: UnitId) -> bool {
        false
    }
    /// Whether the monster data has an NPC interaction block.
    fn has_interaction_block(&self, unit: UnitId) -> bool {
        false
    }
    /// Whether `player` is in the NPC's interaction list.
    fn in_interaction_list(&self, npc: UnitId, player: UnitId) -> bool {
        false
    }
    /// The unit's path target unit.
    fn path_target(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// Sets the path facing.
    fn set_facing(&mut self, unit: UnitId, dir: i32) {}
    /// The monster footprint test for `class` at (x, y) in `room`; `false`
    /// (no room) by default.
    fn footprint_ok(&self, game: &Game, class: i32, room: Option<RoomId>, x: i32, y: i32) -> bool {
        false
    }
    /// SandRaider's help search (target scan 1).
    fn nearest_evil_monster(&mut self, game: &mut Game, unit: UnitId) -> Option<UnitId> {
        None
    }

    // ---- the Act II-V AI bodies ([`crate::monsters::ai::AiActs`]) ----

    /// Whether `b` is hostile to `a`.
    fn ai_hostile(&self, game: &Game, a: UnitId, b: UnitId) -> bool {
        false
    }
    /// The unit's owner.
    fn ai_owner(&self, game: &Game, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// A monster's owner record (type, GUID).
    fn ai_owner_record(&self, unit: UnitId) -> Option<(i32, u32)> {
        None
    }
    /// A quest flag on the player's quest record of the difficulty.
    fn ai_quest_flag(&self, player: UnitId, difficulty: u8, quest: i32, flag: i32) -> bool {
        false
    }
    /// The player's portal GUID.
    fn ai_portal_guid(&self, player: UnitId) -> Option<u32> {
        None
    }
    /// Monster data `nComponent[i]`.
    fn ai_component(&self, unit: UnitId, i: usize) -> u8 {
        0
    }
    /// The unit's target unit.
    fn ai_target_unit(&self, game: &Game, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// The monstats chain byte of `class`.
    fn ai_chain_index(&self, class: i32) -> i32 {
        0
    }
    /// The class to use in the room's level; `class` itself by default.
    fn ai_class_for_level(&self, game: &Game, room: Option<RoomId>, class: i32) -> i32 {
        class
    }
    /// Skill level with bonus of the unit's (highest) entry.
    fn ai_skill_level(&self, unit: UnitId, skill: i32, highest: bool) -> Option<i32> {
        None
    }
    /// The unit's highest skill entry: id and mode.
    fn ai_skill_entry(&self, unit: UnitId, skill: i32) -> Option<(i32, u8)> {
        None
    }
    /// Left / right skill: id and level.
    fn ai_hand_skill(&self, unit: UnitId, right: bool) -> Option<(i32, i32)> {
        None
    }
    /// Adds `skill` at `level` as the unit's right skill.
    fn ai_add_right_skill(&mut self, game: &mut Game, unit: UnitId, skill: i32, level: i32) {}
    /// Assigns `skill` at `level` to the unit.
    fn ai_assign_skill(&mut self, game: &mut Game, unit: UnitId, skill: i32, level: i32) {}
    /// Sets the skill entry's parameter; false (no entry) by default.
    fn ai_set_skill_param(&mut self, unit: UnitId, skill: i32, value: i32) -> bool {
        false
    }
    /// Checks whether the unit can use `skill` on `target` or at (x, y).
    fn ai_skill_check(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        skill: i32,
        target: Option<UnitId>,
        x: i32,
        y: i32,
    ) -> bool {
        false
    }
    /// Searches for a corpse for `skill` at `level`.
    fn ai_corpse_search(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        target: Option<UnitId>,
        skill: i32,
        level: i32,
    ) -> Option<UnitId> {
        None
    }
    /// The path pattern.
    fn ai_path_pattern(&self, unit: UnitId) -> i32 {
        0
    }
    /// Sets the path pattern.
    fn ai_set_path_pattern(&mut self, unit: UnitId, pattern: i32) {}
    /// Places the unit at (x, y) in `room`; false by default.
    fn ai_place_unit(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        room: Option<RoomId>,
        x: i32,
        y: i32,
    ) -> bool {
        false
    }
    /// Stamps a collision pattern at (x, y) in `room`.
    fn ai_stamp_pattern(
        &mut self,
        game: &mut Game,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        pattern: i32,
        mask: u16,
    ) {
    }
    /// Clears collision bits of one cell.
    fn ai_clear_cell(&mut self, game: &mut Game, room: Option<RoomId>, x: i32, y: i32, bits: u16) {}
    /// Whether the cell at (x, y) collides with `mask`.
    fn ai_point_collides(
        &self,
        game: &Game,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        mask: u16,
    ) -> bool {
        false
    }
    /// Whether a given pattern collides with `mask` at the unit's position.
    fn ai_pattern_collides(&self, game: &Game, unit: UnitId, pattern: i32, mask: u16) -> bool {
        false
    }
    /// A free point of `size` near (x, y) in `room`.
    fn ai_free_point(
        &mut self,
        game: &mut Game,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        size: i32,
    ) -> Option<(i32, i32)> {
        None
    }
    /// A free spot for a monster of `class` near a start point.
    fn ai_free_spot_for(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        class: i32,
        x: i32,
        y: i32,
    ) -> Option<(i32, i32)> {
        None
    }
    /// The room containing (x, y).
    fn ai_room_at(&self, game: &Game, unit: UnitId, x: i32, y: i32) -> Option<RoomId> {
        None
    }
    /// Moves the unit in a radius around `target` in `mode`.
    fn ai_move_in_radius(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        target: UnitId,
        mode: u8,
        a: i32,
        b: i32,
    ) -> bool {
        false
    }
    /// Sets the unit's path target unit.
    fn ai_set_path_target(&mut self, unit: UnitId, target: UnitId) {}
    /// Path type 2 and compute toward the target; whether it has points.
    fn ai_path_has_points(&mut self, game: &mut Game, unit: UnitId, target: UnitId) -> bool {
        false
    }
    /// The direction (in 64 steps) from the unit toward `target`.
    fn ai_direction64(&self, unit: UnitId, target: UnitId) -> i32 {
        0
    }
    /// Spawns a monster; nothing spawned by default.
    #[allow(clippy::too_many_arguments)]
    fn ai_spawn_monster(
        &mut self,
        game: &mut Game,
        room: RoomId,
        x: i32,
        y: i32,
        class: i32,
        mode: u8,
        spread: i32,
        flags: u32,
    ) -> Option<UnitId> {
        None
    }
    /// SandMaggotQueen's spawn class; -1 by default.
    fn ai_queen_spawn_class(&self, unit: UnitId) -> i32 {
        -1
    }
    /// Kills the unit with `killer` as the attacker.
    fn ai_kill(&mut self, game: &mut Game, unit: UnitId, killer: Option<UnitId>) {}
    /// The unit leaves its room and is removed.
    fn ai_remove_unit(&mut self, game: &mut Game, unit: UnitId) {}
    /// Baal clone owner links.
    fn ai_link_clone(&mut self, game: &mut Game, unit: UnitId, clone: UnitId) {}
    /// Reinitializes the unit as `class` in `mode`.
    fn ai_reinit_class(&mut self, game: &mut Game, unit: UnitId, class: i32, mode: u8) {}
    /// BaalThrone's change-class stat list.
    fn ai_change_class_list(&mut self, game: &mut Game, unit: UnitId, class: i32) {}
    /// WillOWisp's magic-find stat list.
    fn ai_wisp_buff(&mut self, game: &mut Game, target: UnitId, value: i32, expire: i32) {}
    /// WillOWisp's unit find; nothing found by default.
    fn ai_wisp_find(&mut self, game: &mut Game, unit: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    /// BaalThrone wave record: (mapped superunique, class).
    fn ai_wave(&self, w: i32) -> Option<(i32, i32)> {
        None
    }
    /// Clears the room's portal flag.
    fn ai_clear_room_portal_flag(&mut self, game: &mut Game, room: Option<RoomId>) {}
    /// A quest call of the Act II–V bodies; false by default.
    fn ai_quest_call(&mut self, game: &mut Game, unit: UnitId, call: QuestCall) -> bool {
        false
    }
    /// Records the unit's last attacker.
    fn set_last_attacker(&mut self, defender: UnitId, attacker: UnitId) {}
    /// Called after a monster's hit.
    fn monster_hit_hook(&mut self, attacker: UnitId) {}
    /// Called after a non-lethal hit on a monster.
    fn monster_damaged_hook(&mut self, defender: UnitId) {}
    /// Unique-mod hook for a missile's monster owner.
    fn unique_mod_missile(&mut self, game: &mut Game, owner: UnitId, missile: UnitId) {}

    // ---- AI targets and skills -----------------------------------

    /// The game's monster target lists.
    fn target_nodes(&self, game: &Game) -> crate::game::TargetLists {
        Default::default()
    }
    /// The secondary target search.
    fn secondary_target(&mut self, game: &mut Game, unit: UnitId) -> (Option<UnitId>, i32, bool) {
        (None, 0x7FFF_FFFF, false)
    }
    /// The candidates of target scan 6 before the callback's filter: the
    /// live units hostile to `unit`, in scan order; `None` = the host runs the whole search itself
    /// ([`Self::secondary_target`]).
    fn secondary_candidates(&mut self, game: &mut Game, unit: UnitId) -> Option<Vec<UnitId>> {
        None
    }
    /// The nearest player: the NPC itself when none.
    fn nearest_player(&mut self, game: &mut Game, unit: UnitId) -> (UnitId, bool) {
        (unit, false)
    }
    /// Target scan 8: a door.
    fn find_door(&mut self, game: &mut Game, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// objects.txt `MonsterOK`.
    fn door_monster_ok(&self, door: UnitId) -> bool {
        false
    }
    /// Target scan 11: the special walk target.
    fn special_walk_target(&mut self, game: &mut Game, unit: UnitId) -> Option<(UnitId, i32)> {
        None
    }
    // ---- AI quest calls -------------------------------------------

    /// Portal coordinates set up; `true` (nothing to report) by default.
    fn portal_setup(&mut self, game: &mut Game, unit: UnitId, npc: PortalNpc) -> bool {
        true
    }
    fn spawn_town_portal(&mut self, game: &mut Game, unit: UnitId, npc: PortalNpc) {}
    /// The out-of-town portal; `false` (none spawned) by default.
    fn spawn_outside_portal(&mut self, game: &mut Game, unit: UnitId, npc: PortalNpc) -> bool {
        false
    }
    fn portal_coords(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        npc: PortalNpc,
    ) -> Option<(i32, i32)> {
        None
    }
    fn drehya_update(&mut self, game: &mut Game) {}
    fn drehya_wait(&mut self, game: &mut Game) -> bool {
        false
    }
    /// The Npc class cases; defaults: no quest state. Jerhyn without a
    /// palace spawn (chain 11 extra byte 0x0D = 0: a fresh or act-1-done
    /// character) gives palace-active = 1 and (a, b) = (1, 0), so his think
    /// goes on to the interaction step. These defaults are only the
    /// no-quest-host fallback, equal to the no-record case; with a quest
    /// host the palace states are read (`q4::jerhyn_palace_active`).
    fn jerhyn_palace_active(&mut self, game: &mut Game) -> bool {
        true
    }
    fn jerhyn_npc_state(&mut self, game: &mut Game, unit: UnitId) -> (i32, i32) {
        (1, 0)
    }
    fn guard_moving(&mut self, game: &mut Game, unit: UnitId) -> bool {
        false
    }
    fn alkor_bird(&mut self, game: &mut Game) -> bool {
        false
    }
    fn alkor_reset(&mut self, game: &mut Game) {}
    fn ormus_altar(&mut self, game: &mut Game) -> Option<(i32, i32)> {
        None
    }
    fn ormus_set_altar_mode(&mut self, game: &mut Game) {}
    fn cain_town_coords(&mut self, game: &mut Game, unit: UnitId) -> Option<(i32, i32)> {
        None
    }
    fn cain_in_town_activated(&mut self, game: &mut Game, unit: UnitId) {}
    fn anya_open_portal(&mut self, game: &mut Game, unit: UnitId) {}

    /// Current skill := `skill` (unit skill list); false when out of range.
    fn set_current_skill(&mut self, unit: UnitId, skill: i32) -> bool {
        false
    }
    /// Sets the used skill to none.
    fn clear_current_skill(&mut self, unit: UnitId) {}
    /// A missile parameter record's init callback with an id that has no
    /// body here (the others run in `missiles::init_cb`).
    fn missile_init_callback(&mut self, game: &mut Game, missile: UnitId, callback: u32, arg: u32) {
    }
    /// The curse helper.
    #[allow(clippy::too_many_arguments)]
    fn curse(
        &mut self,
        target: UnitId,
        owner: UnitId,
        state: u16,
        stat: u16,
        value: i32,
        frames: i32,
        skill: i32,
        level: i32,
    ) {
    }
    /// Thorns.
    fn thorns(&mut self, a: UnitId, d: UnitId, record: &mut crate::combat::DamageRecord) {}

    // ---- unit events, reaction, overlays ---------------------------

    /// Runs unit event `event` for a host without the event registry: with [`ActionHooks::unit_events`] set
    /// (`ActionHooks::enable_unit_events`, a host with
    /// [`crate::wiring::interaction::UseRest`]) the iteration runs
    /// [`crate::combat::events::run`] on [`ActionHooks::handlers`] and this
    /// seam is not called. Event 0 (hit by missile) comes with no record
    /// and possibly no unit.
    fn unit_event(
        &mut self,
        event: u8,
        unit: Option<UnitId>,
        other: Option<UnitId>,
        record: Option<&mut crate::combat::DamageRecord>,
    ) {
    }
    /// The global layer split (shift, mask) of the item event
    /// registrations, used when the hooks have no body tables. Default:
    /// (6, 0x3F).
    fn event_layer_split(&self) -> (u32, u32) {
        (6, 0x3F)
    }
    /// Terror install on `unit` from `source` with `skill` and arguments
    /// `a`, `b`.
    fn event_terror(
        &mut self,
        game: &mut Game,
        source: UnitId,
        unit: UnitId,
        skill: i32,
        a: i32,
        b: i32,
    ) {
    }
    /// Whether the unit's collision pattern at `at` in its room has no bits
    /// of the unit's collision mask.
    fn event_point_free(&self, unit: UnitId, at: (i32, i32)) -> bool {
        false
    }
    /// The corpse find of item target 3: the first unit of the unit find
    /// around `t0`, radius 10, flags 0x1002, with the corpse filter.
    fn event_corpse_near(&mut self, game: &mut Game, t0: UnitId) -> Option<UnitId> {
        None
    }
    /// Queues S->C 0x99 / 0x9A on the unit's message list and the update
    /// queue.
    fn queue_item_cast(&mut self, unit: UnitId, msg: crate::combat::events::ItemCastMsg) {}
    /// The raise test for `unit` (Reanimate).
    fn raise_test(&self, unit: UnitId) -> bool {
        false
    }
    /// Clears the unit's collision pattern (bits 0x8000) at its position.
    fn clear_pattern(&mut self, unit: UnitId) {}
    /// A step of the Reanimate raise on the new monster with no body in
    /// d2-sim.
    fn raise_step(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        step: crate::combat::events::RaiseStep<UnitId>,
    ) {
    }

    // ---- player death ([`super::death`]) ---------------------------

    /// The stash gold limit.
    fn stash_cap(&self, unit: UnitId) -> i32 {
        0
    }
    /// The death's gold drop of `amount` from the player.
    fn death_drop_gold(&mut self, game: &mut Game, unit: UnitId, amount: i32) {}
    /// Corpse creation without its experience: a new player-type unit of the
    /// player's class in mode 17 holding the player's items. Default: none.
    fn create_corpse(&mut self, game: &mut Game, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// The corpse's owner GUID from its inventory.
    fn corpse_owner_guid(&self, corpse: UnitId) -> Option<u32> {
        None
    }
    /// Whether `unit` may loot the corpse of its player owner.
    fn corpse_loot_allowed(&self, corpse: UnitId, unit: UnitId) -> bool {
        false
    }

    /// The first step of the hit reaction (the town rule); the remaining
    /// steps run in [`super::reaction::reaction`].
    fn reaction(&mut self, a: UnitId, d: UnitId, record: &mut crate::combat::DamageRecord) {}
    /// AI state setter (the monster data's `dwAiState`); read back by
    /// [`Self::ai_state`].
    fn set_monster_ai_state(&mut self, unit: UnitId, v: u32) {}
    /// A player mode request (re-entry 0) on a host without the path
    /// provider. Default: nothing.
    fn player_mode_request(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        skill: Option<u16>,
        mode: u32,
        target: crate::path::walk::request::WalkTarget,
    ) {
    }
    /// Starts overlay `id` on the unit.
    fn overlay(&mut self, unit: UnitId, id: i32) {}
    /// Refreshes the unit's animation rate.
    fn refresh_anim_rate(&mut self, unit: UnitId) {}
    /// Monster sound: queues the unit's sound slot (flag 0x400); the unit
    /// update sends it as S->C 0x2C (the NPC greeting is event 18).
    fn play_sound(&mut self, game: &mut Game, unit: UnitId, sound: u32, to: Option<UnitId>) {
        let _ = crate::units::sound::queue_sound(game, unit, sound as u16, to);
    }
    /// Operates a door in a game without an object state (with one:
    /// [`super::objects`]).
    fn operate_door(&mut self, game: &mut Game, unit: UnitId, door: UnitId) {}

    // ---- items ----------------------------------------------------

    /// A stat of the unit with `layer` (the item and skill getter).
    fn item_stat(&self, unit: UnitId, stat: u16, layer: u16) -> i32 {
        0
    }
    /// The unit's skill list.
    fn skill_list(&self, unit: UnitId) -> Vec<crate::skills::SkillEntry> {
        Vec::new()
    }
    /// The unit's used skill entry.
    fn used_skill(&self, unit: UnitId) -> Option<crate::skills::SkillEntry> {
        None
    }
    /// The weapon the unit currently holds.
    fn current_weapon(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// The current weapon chosen by the used skill's `weapsel`; `flag` is the used entry's bit 0x2000. `Some(None)`: no
    /// damage weapon (a dual-capable class's weapsel 4). Default: not
    /// answered ([`Pending::current_weapon`]).
    fn damage_weapon(&self, unit: UnitId, weapsel: u8, flag: bool) -> Option<Option<UnitId>> {
        None
    }
    /// The unit's weapon.
    fn weapon(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    fn item_at(&self, unit: UnitId, loc: u8) -> Option<UnitId> {
        None
    }
    fn item_is(&self, item: UnitId, itype: i32) -> bool {
        false
    }
    fn itype_is(&self, itype: i32, parent: i32) -> bool {
        false
    }
    fn wield_type(&self, item: UnitId) -> i32 {
        0
    }
    fn item_damage(&self, item: UnitId, max: bool) -> i32 {
        0
    }
    fn str_dex_bonus(&self, item: UnitId) -> (i32, i32) {
        (0, 0)
    }
    fn item_flag_throw(&self, item: UnitId) -> bool {
        false
    }
    /// The items-row half of the durability test (`nodurability` = 0,
    /// `durability` non-zero); the stat half is [`super::View::item_has_durability`].
    fn item_has_durability(&self, item: UnitId) -> bool {
        false
    }
    fn durability_loss(&mut self, owner: UnitId, item: UnitId) {}
    /// Whether the unit has a shield.
    fn has_shield(&self, unit: UnitId) -> bool {
        false
    }
    /// Whether the unit has a composit shield.
    fn composit_shield(&self, unit: UnitId) -> bool {
        false
    }
    /// The unit's weapon class.
    fn weapon_class(&self, unit: UnitId) -> i32 {
        0
    }
    /// The unit's weapon hit class.
    fn weapon_hit_class(&self, unit: UnitId) -> u32 {
        0
    }
    /// Switches the dual-wield state of `a` (off-hand or main hand) on or off.
    fn dual_wield_switch(&mut self, a: UnitId, offhand: bool, on: bool) {}
    /// The attack weapon of a player, and of a monster. Default:
    /// [`Self::weapon`].
    fn attack_weapon(&self, unit: UnitId) -> Option<UnitId> {
        self.weapon(unit)
    }
    /// Whether the item's `2handed` is non-zero. Default: false.
    fn item_two_handed(&self, item: UnitId) -> bool {
        false
    }
    /// Toggles the dual-wield stat. Default: nothing.
    fn dual_wield_toggle(&mut self, unit: UnitId) {}

    // ---- objects, interaction, messages ----------------------------

    /// The object mode change of an object without object data (with data:
    /// [`super::objects`]).
    fn set_object_mode(&mut self, game: &mut Game, object: UnitId, mode: u8) {}
    /// The waypoint hostile-delay test from the host clock (never true in
    /// single player).
    fn hostile_delay(&self, player: UnitId) -> bool {
        false
    }
    /// Attaches sound `event` to the unit.
    fn attach_sound(&mut self, unit: UnitId, event: u8) {}
    /// A message to the player's client (server transport).
    fn send(&mut self, player: UnitId, msg: &[u8]) {}
    /// Warps the player (level change or act change, and placement).
    fn warp(&mut self, game: &mut Game, player: UnitId, level: u32, tile_code: u8) {}
    /// Sets the player to mode 2 (no skill) at arrival.
    fn set_player_mode_arrival(&mut self, game: &mut Game, player: UnitId) {}

    // ---- objects (the `ObjectWorld` seams) -----------------------

    /// The key test and use (inventory): whether the player carries a key;
    /// when it does, the host uses one up ([`KEY_USE_MARK`]). Default: no
    /// key.
    fn object_key_test(&mut self, player: UnitId) -> bool {
        false
    }
    /// Whether the operator is in interact range of the object. Default: out
    /// of range (the operate entry then returns before the dispatch).
    fn object_in_range(&self, game: &Game, operator: UnitId, object: UnitId) -> bool {
        false
    }
    /// Whether the player is busy (a player data field is non-zero).
    fn object_player_busy(&self, player: UnitId) -> bool {
        false
    }
    /// An item is on the player's cursor (inventory).
    fn object_cursor_item(&self, player: UnitId) -> bool {
        false
    }
    /// The act II staff-tomb level. Default: a
    /// level id no level has.
    fn object_staff_tomb(&self) -> u32 {
        u32::MAX
    }
    /// The portal travel's host facts and steps
    /// ([`crate::world::objects::MiscWorld`]): the party id (default 0xFFFF,
    /// none).
    fn object_party_id(&self, unit: UnitId) -> u16 {
        0xFFFF
    }
    /// The partner portal. Default: none.
    fn object_portal_partner(&mut self, game: &mut Game, object: UnitId) -> Option<UnitId> {
        None
    }
    /// The player's quest record for the difficulty exists. Default: no.
    fn object_quest_record(&self, player: UnitId) -> bool {
        false
    }
    /// A bit of quest `quest` in the player's quest record. Default: clear.
    fn object_quest_bit(&self, player: UnitId, quest: u32, bit: u8) -> bool {
        false
    }
    /// The player's portal GUID. Default 0.
    fn object_portal_guid(&self, player: UnitId) -> u32 {
        0
    }
    /// The spawn point of `level`. Default: none.
    fn object_level_spawn(&mut self, game: &mut Game, level: u32) -> Option<(RoomId, i32, i32)> {
        None
    }
    /// Quest event 3 (CHANGEDLEVEL) from `from` to `to`. Default: nothing.
    fn object_quest_level_change(&mut self, player: UnitId, from: u32, to: u32) {}
    /// A portal's removal. Default: nothing.
    fn object_remove_portal(&mut self, game: &mut Game, object: UnitId) {}
    /// The Act V hook of the partner portal. Default: nothing.
    fn object_portal_act5(&mut self, partner: UnitId) {}
    /// The Act V hook of a town portal the cast opened. Default: nothing.
    fn object_portal_opened(&mut self, portal: UnitId) {}
    /// State 102 on the player until `expire`. Default: nothing.
    fn object_just_portaled(&mut self, game: &mut Game, player: UnitId, expire: i32) {}
    /// What the object module handed back without running it: quest,
    /// waypoint and `todo` inits, operates and events, uncovered presets
    /// ([`super::objects::ObjectRoute`]).
    fn object_route(&mut self, game: &mut Game, route: super::objects::ObjectRoute) {}
    /// A class-59 portal's owner for S->C 0x82: (owner GUID, owner name, the paired portal's GUID or
    /// 0xFFFF_FFFF). Default: unknown (nothing is sent).
    fn portal_owner(&self, object: UnitId) -> Option<(u32, Vec<u8>, u32)> {
        None
    }
    /// The skill messages 0x21 of a monster's add: for each `monstats`
    /// skill slot i = 0..7 whose `SendSkills` bit i is set, whose skill id
    /// is valid and which the unit has: (skill, base level, bonus level).
    /// Default: none.
    fn monster_add_skills(&self, unit: UnitId) -> Vec<(u16, u8, u8)> {
        Vec::new()
    }
    /// The inventory messages of `unit` to `receiver`'s client. Default:
    /// nothing.
    fn inventory_messages(&mut self, receiver: UnitId, unit: UnitId) {}
    /// The GUID of the unit's minion owner when it is a player and the
    /// unit's pet type is 7 (hireling). Default: none.
    fn hireling_owner_guid(&self, unit: UnitId) -> Option<u32> {
        None
    }
    /// The unit's owner fields (type, id). Default: none.
    fn unit_owner(&self, unit: UnitId) -> Option<(u32, u32)> {
        None
    }
    /// The C->S 0x13 object case's reach step: distance > 50 refuses; in
    /// range and unobstructed stops the player and operates; else the player
    /// walks and operates on arrival. Default: operate (the operate entry's own range test,
    /// [`Pending::object_in_range`], still applies).
    fn object_approach(
        &mut self,
        game: &mut Game,
        player: UnitId,
        object: UnitId,
    ) -> super::objects::ObjectReach {
        super::objects::ObjectReach::Operate
    }

    /// The preview's interact reach in sub-tiles (the Chebyshev distance
    /// of the path positions), for a host whose range test
    /// and walk step are not settled. `Some(r)`: the 0x13
    /// object case refuses beyond 50, does nothing
    /// beyond `r` (the client walks and sends 0x13 again on arrival) and
    /// operates within it; [`Pending::object_in_range`] is then this
    /// test. Default: none (the seams above decide).
    /// D2RUST's own (play preview).
    fn object_preview_range(&self) -> Option<i32> {
        None
    }

    // ---- the kill and the death ----------------------------------

    /// Monster init's quest chain link. Default: none.
    fn monster_quest_chain(&mut self, unit: UnitId, chain: u32) {}
    /// The quest events queued since the last take, in order. Default:
    /// none.
    fn take_quest_events(&mut self) -> Vec<QuestEvent> {
        Vec::new()
    }
    /// A step of the kill with no written body ([`KillStep`]).
    fn kill_step(&mut self, game: &mut Game, step: KillStep, defender: UnitId, attacker: UnitId) {}
    /// The player credited for a kill by a monster attacker: its owner, or
    /// the owner of a flag-0x800 stat list on the attacker, then the
    /// defender. Default: none (no experience).
    fn kill_credited_player(&self, attacker: UnitId, defender: UnitId) -> Option<UnitId> {
        None
    }
    /// Player `p`'s hireling share of the kill experience. Default:
    /// nothing.
    fn kill_hireling_share(&mut self, p: UnitId, attacker: UnitId, defender: UnitId, e: i32) {}
    /// Whether the player's party id is not 0xFFFF. Default: not in a party
    /// (single player).
    fn kill_in_party(&self, p: UnitId) -> bool {
        false
    }
    /// The party share's kept members. Default: `p` only.
    fn kill_party_members(&self, p: UnitId, defender: UnitId) -> Vec<UnitId> {
        vec![p]
    }
    /// The monster's superunique index (hcIdx other than -1).
    fn superunique(&self, unit: UnitId) -> Option<u16> {
        None
    }
    /// The unit's minion owner.
    fn minion_owner(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// The party count the drop walk reads for `unit`.
    fn party_size(&self, unit: UnitId) -> Option<i32> {
        None
    }
    /// The quest treasure class test for the quest owner `p` (resolved by
    /// the caller): `p` is a player whose quest flags for the difficulty of
    /// quest `quest` have none of 15, 1 and `cp`. False: no quest TC.
    fn quest_tc_open(&self, p: UnitId, quest: u8, cp: u8) -> bool {
        false
    }
    /// The players' quest flags for the game's difficulty, published by
    /// the quest control once per tick (for [`Self::quest_tc_open`]).
    /// Default: nothing.
    fn publish_quest_flags(&mut self, flags: &[(UnitId, crate::world::quests::QuestFlags)]) {}

    // ---- vitals (the rest of `VitalsRest`) -----------------------

    /// Refreshes the unit after a strength / dexterity change.
    fn stats_refresh(&mut self, unit: UnitId) {}
    /// The level-up notification (party roster, sound, broadcast,
    /// callbacks).
    fn level_up_notify(&mut self, unit: UnitId) {}
    /// Unit event 12 `levelup` (event registry).
    fn level_up_event(&mut self, unit: UnitId) {}

    /// The monster death start, run by the monster mode set with the mode
    /// change's `target` (`ActionHooks::mode_target`): mode 0, the death
    /// clean-up ([`super::monster_death::death_cleanup`]), then the drop gate
    /// and drop (`crate::wiring::economy::monster_death_drop`) and the
    /// evil-killed count. A host that holds the economy state overrides
    /// this; when it does not call the clean-up, the wiring runs it after
    /// this returns. The room's dead-GUID ring is the wiring's. Returns
    /// whether the mode started; default: started, nothing done (as every
    /// other start function).
    fn monster_death_start(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        unit: UnitId,
        target: Option<UnitId>,
    ) -> bool
    where
        Self: Sized,
    {
        true
    }

    // ---- the monster mode message and the death end -------------

    /// Whether the monster type flags have bit 0x100, which sets bit 0x80 of
    /// a mode-3 message's d. Default: false.
    fn monster_flag_100(&self, unit: UnitId) -> bool {
        false
    }
    /// The animation refresh of the stepping death (base id 78).
    fn refresh_animation(&mut self, game: &mut Game, unit: UnitId) {}
    /// The `monstats` `SplEndDeath` action of the death end: 1 reinitializes
    /// the unit as `minion` (mode 1) and then runs a follow-up step; 2 kills
    /// the unit's owner. Neither is implemented here. Default: nothing.
    fn death_end_action(&mut self, game: &mut Game, unit: UnitId, action: u8, minion: u16) {}

    // ---- skill bodies (their other systems) ----------------------

    /// Whether `a` and `b` are allies (the same unit after the monster owner
    /// resolution, or two players in one party). Default: the same unit.
    fn allied(&self, a: UnitId, b: UnitId) -> bool {
        a == b
    }
    /// Whether the player's pet may be unsummoned (the pet check and the
    /// pettype `unsummon` bit).
    fn pet_unsummonable(&self, unit: UnitId, pet: UnitId) -> bool {
        false
    }
    /// Sets parameter `i` (1 or 2) of the used skill entry.
    fn set_entry_param(&mut self, unit: UnitId, i: u8, v: i32) {}
    /// The composit weapon class, through the disguise remap.
    fn composit_weapon_class(&self, unit: UnitId) -> i32 {
        0
    }
    /// A player's hand class.
    fn hand_class(&self, unit: UnitId) -> i32 {
        0
    }
    /// Whether the item's type has a `shoots` value.
    fn item_shoots(&self, item: UnitId) -> bool {
        false
    }
    /// Whether the item is `stackable`.
    fn item_stackable(&self, item: UnitId) -> bool {
        false
    }
    /// The item's maximum stack.
    fn item_max_stack(&self, item: UnitId) -> i32 {
        0
    }
    /// The item's maximum durability. `None`: unknown, the durability is
    /// left as it is.
    fn item_max_durability(&self, item: UnitId) -> Option<i32> {
        None
    }
    /// The quantity-replenish timer of the item.
    fn quantity_timer(&mut self, game: &mut Game, item: UnitId) {}
    /// Attack-mode cleanup.
    fn attack_cleanup(&mut self, unit: UnitId) {}
    /// Weapon cleanup.
    fn weapon_cleanup(&mut self, unit: UnitId) {}
    /// Passive refresh.
    fn passive_refresh(&mut self, unit: UnitId) {}
    /// The buff callback's refresh.
    fn buff_refresh(&mut self, unit: UnitId) {}
    /// A player's skill resync.
    fn skill_resync(&mut self, unit: UnitId) {}
    /// Applied after a passive state is switched on.
    fn passive_state_apply(&mut self, unit: UnitId, entry: &crate::skills::SkillEntry) {}
    /// Sets the unit's AI state `k` from the curse bodies.
    fn set_ai_state(&mut self, unit: UnitId, k: i32) {}
    /// Aura mana under blood mana.
    fn blood_mana(&mut self, unit: UnitId, cost: i32) {}

    // ---- skill bodies, batch 2 and 3 ------------------------------

    /// A call of the bodies into a system with no provider here
    /// ([`crate::skills::use_::bodies::BodyEffect`]). Default: nothing.
    fn body_effect(&mut self, e: crate::skills::use_::bodies::BodyEffect<UnitId, UnitId, RoomId>) {}
    /// A path operation ([`crate::skills::use_::bodies::PathOp`]; paths of
    /// skill moves are not provided). Default: nothing, compute 0.
    fn body_path_op(
        &mut self,
        unit: UnitId,
        op: crate::skills::use_::bodies::PathOp<UnitId>,
    ) -> i32 {
        0
    }
    /// The game-level flag the monster init reads as "L" (one of two game
    /// bytes is non-zero). Default: false.
    fn l_flag(&self) -> bool {
        false
    }
    /// The unit's frame event index. Default: 0.
    fn frame_event_index(&self, unit: UnitId) -> i32 {
        0
    }
    /// Sets the unit's frame event index.
    fn set_frame_event_index(&mut self, unit: UnitId, i: i32) {}
    /// A monster AI control's spawn class. Default: none.
    fn minion_spawn_class(&self, unit: UnitId) -> Option<i32> {
        None
    }
    /// The owner's linked unit. Default: none.
    fn linked_unit(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// The death delay's killer. Default: none.
    fn killer_of(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// The minion owner record (GUID, type). Default: none.
    fn minion_owner_ident(&self, unit: UnitId) -> Option<(u32, u32)> {
        None
    }
    /// Whether a handler with this key and skill field exists. Default:
    /// none.
    fn has_handler(&self, unit: UnitId, key_type: i32, key: i32, skill: i32) -> bool {
        false
    }
    /// Param `i` (1…4) of the unit's skill entry. Default: 0.
    fn entry_param(&self, unit: UnitId, e: &crate::skills::SkillEntry, i: u8) -> i32 {
        0
    }
    /// Param `i` of the unit's skill entry set.
    fn set_entry_param_of(&mut self, unit: UnitId, e: &crate::skills::SkillEntry, i: u8, v: i32) {}
    /// The skill entry's flags. Default: 0.
    fn entry_flags(&self, unit: UnitId, e: &crate::skills::SkillEntry) -> u32 {
        0
    }
    /// Entry flags set.
    fn set_entry_flags(&mut self, unit: UnitId, e: &crate::skills::SkillEntry, f: u32) {}
    /// Sets the skill entry's mode.
    fn set_entry_mode(&mut self, unit: UnitId, e: &crate::skills::SkillEntry, m: u32) {}
    /// The disguise remap of a mode. Default: the mode unchanged.
    fn disguise_mode(&self, unit: UnitId, m: u32) -> u32 {
        m
    }
    /// The used skill's sequence records. Default: none.
    fn skill_sequence(&self, unit: UnitId) -> Option<Vec<[u8; 6]>> {
        None
    }
    /// Rewinds the unit's animation to position `p`.
    fn anim_rewind(&mut self, unit: UnitId, p: i32) {}
    /// Restarts the unit's animation with value `v`.
    fn anim_restart(&mut self, unit: UnitId, v: i32) {}
    /// Continues the unit's animation from frame `f`.
    fn anim_from(&mut self, unit: UnitId, f: i32) {}
    /// The room level's act. Default: 0.
    fn room_act(&self, room: RoomId) -> i32 {
        0
    }
    /// The room level's `Teleport`. Default: no level record.
    fn room_teleport(&self, room: RoomId) -> Option<i32> {
        None
    }
    /// A free point near `at`. Default: none.
    fn free_point(
        &mut self,
        room: RoomId,
        at: (i32, i32),
        size: i32,
        mask: u32,
        fallback: bool,
    ) -> Option<(RoomId, (i32, i32))> {
        None
    }
    /// Whether the unit's pattern collides with `mask` at `at`. Default:
    /// collides.
    fn pattern_collides(&self, room: RoomId, at: (i32, i32), unit: UnitId, mask: u32) -> bool {
        true
    }
    /// Whether a box of `size` collides with `mask` at `at`. Default:
    /// collides.
    fn box_collides(&self, room: RoomId, at: (i32, i32), size: i32, mask: u32) -> bool {
        true
    }
    /// Whether the line from `from` to `to` is blocked by `mask`. Default:
    /// blocked.
    fn body_line_blocked(&self, room: RoomId, from: (i32, i32), to: (i32, i32), mask: u32) -> bool {
        true
    }
    /// Places the unit at `at` in `room`. Default: not placed.
    fn place_unit(&mut self, unit: UnitId, room: Option<RoomId>, at: (i32, i32)) -> bool {
        false
    }
    /// The number of points on the unit's path. Default: 0.
    fn path_point_count(&self, unit: UnitId) -> i32 {
        0
    }
    /// The path's last point. Default: (0, 0).
    fn path_last_point(&self, unit: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// The path's target point. Default: (0, 0).
    fn path_target_point(&self, unit: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// Creates a monster (flags 0x42). Default: none.
    fn create_monster(
        &mut self,
        room: RoomId,
        at: (i32, i32),
        class: i32,
        mode: i32,
        spread: i32,
    ) -> Option<UnitId> {
        None
    }
    /// Monster mode request. Default: 0.
    fn mode_request(&mut self, unit: UnitId, mode: i32, target: Option<UnitId>) -> i32 {
        0
    }
    /// Whether the unit is busy. Default: false.
    fn inventory_busy(&self, unit: UnitId) -> bool {
        false
    }
    /// Whether the unit has an inventory. Default: false.
    fn has_inventory(&self, unit: UnitId) -> bool {
        false
    }
    /// The weapon in use. Default: none.
    fn weapon_in_use(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// The item's body location. Default: 0.
    fn body_loc(&self, item: UnitId) -> i32 {
        0
    }
    /// Whether the item is usable. Default: false.
    fn item_usable(&self, item: UnitId) -> bool {
        false
    }
    /// Whether the item is active. Default: false.
    fn item_active(&self, item: UnitId) -> bool {
        false
    }
    /// The unit's shield. Default: none.
    fn shield(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// armor `mindam` / `maxdam`. Default: no record.
    fn shield_damage(&self, item: UnitId) -> Option<(i32, i32)> {
        None
    }
    /// weapons `missiletype`. Default: 0.
    fn item_missile_type(&self, item: UnitId) -> i32 {
        0
    }
    /// The Iron Golem item test. Default: false.
    fn golem_item(&self, unit: UnitId) -> bool {
        false
    }
    /// The unit's first item location. Default: 0.
    fn item_first_loc(&self, unit: UnitId) -> i32 {
        0
    }
    /// Whether the items in locations 6 and 5 are both `mele`. Default: false.
    fn two_melee_weapons(&self, unit: UnitId) -> bool {
        false
    }
    /// The attack frames of the item. Default: none (a fatal case).
    fn attack_frames(&self, unit: UnitId, item: UnitId) -> Option<i32> {
        None
    }

    // ---- skill bodies, batch 4 -------------------------------------

    /// The 64-step direction from the unit to (x, y). Default: 0.
    fn body_dir64(&self, unit: UnitId, at: (i32, i32)) -> i32 {
        0
    }
    /// A missile's total frames. Default: 0.
    fn body_missile_frames(&self, missile: UnitId) -> i32 {
        0
    }
    /// Sets a missile's total frames and frames left. Default: nothing.
    fn body_set_missile_frames(&mut self, missile: UnitId, total: i32, left: i32) {}
    /// Whether an action event falls in the frames (a, b]. Default: none.
    fn body_action_event_between(&self, unit: UnitId, a: i32, b: i32) -> bool {
        false
    }
    /// The books row of an item's spell index: (`scrollskill`,
    /// `bookskill`). Default: none.
    fn body_book_skills(&self, item: UnitId) -> Option<(i32, i32)> {
        None
    }
    /// The unit's inventory nodes (item, node kind) in list order.
    /// Default: none.
    fn body_inventory_nodes(&self, unit: UnitId) -> Vec<(UnitId, i32)> {
        Vec::new()
    }
    /// The unit find around `at` with radius `r` and flags `f`. Default:
    /// nothing found.
    fn body_unit_find(&self, room: RoomId, at: (i32, i32), r: i32, f: u32) -> Vec<UnitId> {
        Vec::new()
    }
    /// Whether the cell at `at` collides with `mask`. Default: collides.
    fn body_point_collides(&self, room: RoomId, at: (i32, i32), mask: u32) -> bool {
        true
    }
    /// Places the unit at `at` in `room` with a non-zero flag `a`. Default:
    /// not placed.
    fn body_place_unit_flag(
        &mut self,
        unit: UnitId,
        room: Option<RoomId>,
        at: (i32, i32),
        a: i32,
    ) -> bool {
        false
    }
    /// A monster creation entry point. Default: none.
    fn body_spawn_monster(
        &mut self,
        q: crate::skills::use_::bodies::MonsterSpawn<UnitId, RoomId>,
    ) -> Option<UnitId> {
        None
    }
    /// Turns the weapon lists off or back on for the kick damage.
    fn toggle_weapon_lists(&mut self, unit: UnitId, on: bool) {}
    /// The two damage bytes of the boots' items record. Default: (0, 0).
    fn boots_damage(&self, item: UnitId) -> (i32, i32) {
        (0, 0)
    }

    /// The units the unit scan (owner, position, radius `r`, flags `f`,
    /// `noaura`) accepts, in order, for the missile area bodies. The scan
    /// runs on the skill use
    /// view (`UseView`, which needs `UseRest`), so a seam value that also
    /// implements `UseRest` routes it to
    /// [`crate::wiring::interaction::skill_events::missile_area_units`].
    /// Default: none.
    fn missile_area_units(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        owner: UnitId,
        at: (i32, i32),
        r: i32,
        f: u32,
        noaura: bool,
    ) -> Vec<UnitId>
    where
        Self: Sized,
    {
        Vec::new()
    }

    /// The ally test for the missile bodies, on the skill use view
    /// (`BodyWorld::allied`); routed to
    /// [`crate::wiring::interaction::skill_events::missile_ally_test`] by a
    /// [`crate::wiring::interaction::UseRest`] value. Default: false.
    fn missile_ally_test(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, a: UnitId, b: UnitId) -> bool
    where
        Self: Sized,
    {
        false
    }

    /// The shout state (unit, owner, skill, level) for the shout missile
    /// hit. Runs on the skill use view, so a
    /// [`crate::wiring::interaction::UseRest`] value routes it to
    /// [`crate::wiring::interaction::skill_events::missile_shout_state`].
    /// Default: nothing.
    fn missile_shout_state(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        unit: UnitId,
        owner: UnitId,
        skill: i32,
        level: i32,
    ) where
        Self: Sized,
    {
    }

    /// Terror install for the Howl and Grim Ward missile hits. Runs on the skill use view like [`Pending::missile_shout_state`];
    /// a [`crate::wiring::interaction::UseRest`] value routes it to
    /// [`crate::wiring::interaction::skill_events::missile_terror`].
    /// Default: nothing.
    fn missile_terror(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        source: UnitId,
        unit: UnitId,
        b: i32,
    ) where
        Self: Sized,
    {
    }

    /// Skill server-do `index` with `caster` (a missile) as the caster.
    /// Runs on the skill use view, so a
    /// [`crate::wiring::interaction::UseRest`] value routes it to
    /// [`crate::wiring::interaction::skill_events::missile_skill_srv_do`].
    /// Default: nothing.
    fn missile_skill_srv_do(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        caster: UnitId,
        index: i32,
        skill: i32,
        level: i32,
    ) where
        Self: Sized,
    {
    }

    // ---- skill timer events ----------------------------------------

    /// Routes timer events 5, 8 and 9 to the skill use pipeline. The
    /// pipeline's own seams ([`crate::wiring::interaction::UseRest`]) are a
    /// wider bound than `Pending`, so the route is chosen by the seam
    /// value: a value that also implements `UseRest` sets this to
    /// [`crate::wiring::interaction::skill_events::route`]. Default: nothing.
    fn skill_event(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, ev: SkillEvent)
    where
        Self: Sized,
    {
    }

    /// Player event 0 in an attack, cast or skill mode: the action frame
    /// with the event's (a1, a2); returns
    /// the action result (2 runs the ENDANIM handler at once). A seam
    /// value that also implements
    /// [`crate::wiring::interaction::UseRest`] routes it to
    /// [`crate::wiring::interaction::skill_events::action_frame`]. Default:
    /// 1, nothing done.
    fn action_frame(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        unit: UnitId,
        a1: u32,
        a2: u32,
    ) -> u32
    where
        Self: Sized,
    {
        1
    }
    /// The skill start of a monster's attack / skill start and sequence
    /// start; its result. A [`crate::wiring::interaction::UseRest`] value
    /// routes it to
    /// [`crate::wiring::interaction::skill_events::monster_skill_start`].
    /// Default: 0 (no skill pipeline: no used skill).
    fn monster_skill_start(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId) -> i32
    where
        Self: Sized,
    {
        0
    }
    /// `apply_state` for a caller outside the skill pipeline (the monster
    /// umod 7 curse). A
    /// [`crate::wiring::interaction::UseRest`] value routes it to
    /// [`crate::wiring::interaction::skill_events::apply_state_req`].
    /// Default: none (no state is applied).
    fn apply_state_req(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        req: crate::skills::use_::bodies::helpers::StateRequest<UnitId>,
    ) -> Option<crate::stats::ListId>
    where
        Self: Sized,
    {
        None
    }
    /// The join's Iron Golem re-summon: routed
    /// to [`crate::wiring::interaction::skill_events::golem_resummon`] by
    /// a [`crate::wiring::interaction::UseRest`] value. True when a golem
    /// was made. Default: false. D2RUST's own, unverified (REC-265).
    fn golem_resummon(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, player: UnitId) -> bool
    where
        Self: Sized,
    {
        false
    }
    /// A monster's aura as its right skill (umod 30; Duriel). Routed to
    /// [`crate::wiring::interaction::skill_events::monster_right_aura`] by
    /// a [`crate::wiring::interaction::UseRest`] value. Default: nothing.
    fn monster_right_aura(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        unit: UnitId,
        skill: i32,
        level: i32,
    ) where
        Self: Sized,
    {
    }
    /// A monster's skill entry set by the AI init: the passive refresh of
    /// the entry at its base `level`. Routed to
    /// [`crate::wiring::interaction::skill_events::monster_passive_refresh`]
    /// by a [`crate::wiring::interaction::UseRest`] value. Default: nothing.
    fn monster_passive_refresh(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        unit: UnitId,
        skill: i32,
        level: i32,
    ) where
        Self: Sized,
    {
    }
    /// The Bone Wall maker's `summon_class`: routed to
    /// [`crate::wiring::interaction::skill_events::missile_summon_class`]
    /// by a [`crate::wiring::interaction::UseRest`] value. Default: none.
    fn missile_summon_class(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        owner: UnitId,
        skill: i32,
        level: i32,
    ) -> (i32, i32)
    where
        Self: Sized,
    {
        (-1, 0)
    }
    /// The skill handlers of stats 83, 126, 127, 188 (refresh all skills):
    /// routed to
    /// [`crate::wiring::interaction::skill_events::skill_stat_refresh`] by
    /// a [`crate::wiring::interaction::UseRest`] value. Default: nothing.
    fn skill_stat_refresh(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        unit: UnitId,
        stat: u16,
        layer: u16,
    ) where
        Self: Sized,
    {
    }
    /// The Bone Wall maker's summon spawn (flags 0xD).
    /// Default: none.
    #[allow(clippy::too_many_arguments)]
    fn missile_summon_spawn(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        owner: UnitId,
        class: i32,
        mode: i32,
        at: (i32, i32),
        pet_type: i32,
    ) -> Option<UnitId>
    where
        Self: Sized,
    {
        None
    }
    /// The Bone Wall maker's piece binding. Default: nothing.
    #[allow(clippy::too_many_arguments)]
    fn missile_bone_wall_piece(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        owner: UnitId,
        anchor: UnitId,
        piece: UnitId,
        skill: i32,
        level: i32,
    ) where
        Self: Sized,
    {
    }
    /// The save load's right-skill aura start: routed to
    /// [`crate::wiring::interaction::skill_events::right_aura_select`] by
    /// a [`crate::wiring::interaction::UseRest`] value. Default: nothing.
    fn right_aura_select(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, player: UnitId)
    where
        Self: Sized,
    {
    }
    /// The save load's passive states (the passive part of the skill
    /// assign): routed to
    /// [`crate::wiring::interaction::skill_events::passive_refresh_all`]
    /// by a [`crate::wiring::interaction::UseRest`] value. Default:
    /// nothing.
    fn passive_refresh_all(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId)
    where
        Self: Sized,
    {
    }
    /// The passive states' unit bits, turned on at the end of the join
    /// (after the player's 0xAA; routed to
    /// [`crate::wiring::interaction::skill_events::passive_states_on`]).
    /// Default: nothing.
    fn passive_states_on(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId)
    where
        Self: Sized,
    {
    }
    /// The pet follow of the summoned pet types: routed to
    /// [`crate::wiring::interaction::summon::summon_follow`] by a
    /// [`crate::wiring::interaction::UseRest`] value. Default: nothing.
    fn summon_follow(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, player: UnitId)
    where
        Self: Sized,
    {
    }
    /// The save load's right-skill aura (as for C->S 0x3C SelectSkill):
    /// routed to
    /// [`crate::wiring::interaction::skill_events::assign_right_aura`] by a
    /// [`crate::wiring::interaction::UseRest`] value. Default: nothing.
    fn assign_right_aura(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId)
    where
        Self: Sized,
    {
    }
    /// The skill part of the monster sequence event 0 (before the animation
    /// refresh): E flags, the moving skill's step and the skill do by frame
    /// code. A
    /// [`crate::wiring::interaction::UseRest`] value routes it to
    /// [`crate::wiring::interaction::skill_events::monster_sequence_frame`].
    /// Default: nothing.
    fn monster_sequence_frame(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId)
    where
        Self: Sized,
    {
    }
    /// The used-skill branch of the monster attack-family event 0 (before
    /// the animation refresh): a moving skill's step and the skill do on
    /// every event. A [`crate::wiring::interaction::UseRest`] value
    /// routes it to
    /// [`crate::wiring::interaction::skill_events::monster_attack_skill`].
    /// Default: nothing.
    fn monster_attack_skill(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId)
    where
        Self: Sized,
    {
    }
    /// The strike of the monster attack-family event 0 with no used skill:
    /// the mode missile (`moving`: its argument), else the melee set-up and
    /// `apply_melee` on the path target unit. A
    /// [`crate::wiring::interaction::UseRest`] value routes it to
    /// [`crate::wiring::interaction::skill_events::monster_attack_strike`].
    /// Default: nothing.
    fn monster_attack_strike(
        h: &mut ActionHooks<Self>,
        sim: &mut Sim<'_>,
        unit: UnitId,
        moving: bool,
    ) where
        Self: Sized,
    {
    }
    /// The monster mode damage of the mode set: the base list's damage and
    /// to-hit for the requested mode. A
    /// [`crate::wiring::interaction::UseRest`] value routes it to
    /// [`crate::wiring::interaction::skill_events::monster_mode_damage`].
    /// Default: nothing.
    fn monster_mode_damage(h: &mut ActionHooks<Self>, sim: &mut Sim<'_>, unit: UnitId, mode: u32)
    where
        Self: Sized,
    {
    }

    // ---- client intents (d2-server's `handlers::player`) ----------------

    /// A test of the overhead text (C->S 0x14): true ends the handler with
    /// nothing done. What it tests is not known. Default: false (the text
    /// passes).
    fn overhead_text_test(&self, text: &[u8]) -> bool {
        false
    }
    /// Replaces the player's overhead record (the old one freed, a new one
    /// with `text` and language byte `byte8`). Its timeout frame is
    /// `UnitRecord::hover`, which the caller sets; the record's other
    /// contents (for the S->C 0x26 message) live here. Default: nothing
    /// kept.
    fn replace_overhead(&mut self, player: UnitId, text: &[u8], byte8: u8, end: i32) {}
    /// The items refresh of the first tick step (an act's environment
    /// report, before its 0x53). Default: nothing.
    fn environment_refresh_items(&mut self, player: UnitId) {}
    /// The room clean-up's client part for a player (a client record field
    /// is reset to 0). Default: nothing (no client record model).
    fn client_cleanup(&mut self, player: UnitId) {}
    /// The text and language byte of the unit's overhead record, for the
    /// overhead 0x26. Default: none kept (nothing is sent).
    fn overhead_record(&self, unit: UnitId) -> Option<(Vec<u8>, u8)> {
        None
    }
    /// Whether a's relation to b has `flag` (party / hostility flags).
    /// Default: no relation.
    fn player_relation(&self, a: UnitId, b: UnitId, flag: u32) -> bool {
        false
    }
    /// The door highlight of C->S 0x3D for the door with `guid`. Default:
    /// nothing.
    fn highlight_door(&mut self, game: &mut Game, player: UnitId, guid: u32) {}
    /// Client flag 4 (hardcore) of the player's client. Default: softcore.
    fn client_hardcore(&self, player: UnitId) -> bool {
        false
    }
    /// Drops the player's client for `reason` (session code). Default:
    /// nothing.
    fn drop_client(&mut self, player: UnitId, reason: u32) {}
    /// The SetStat message part of a stat change (S->C 0x1D-0x1F, after the
    /// stat is set). Default: nothing sent.
    fn stat_sent(&mut self, player: UnitId, stat: u16, value: u32) {}
    /// C->S 0x41's last step: the left skill re-selected (flag 1), then the
    /// right skill (flag 0); the skill list (`UseRest`) owns it. Default:
    /// nothing.
    fn reselect_hand_skills(&mut self, game: &mut Game, player: UnitId) {}
    /// Whether the player is trading (C->S 0x44). Default: not trading.
    fn player_trading(&self, player: UnitId) -> bool {
        false
    }
    /// The staff-in-orifice step for the item and its result (C->S 0x44).
    /// `None` (the default): no provider, the id stays a stub.
    fn staff_in_orifice(
        &mut self,
        game: &mut Game,
        player: UnitId,
        object: u32,
        item: u32,
        action: u16,
    ) -> Option<u32> {
        None
    }
    /// Quest warp gate for a warp from `source` level to `level`: non-zero
    /// blocks. Default: open.
    fn warp_quest_gate(&self, source: u32, level: u32) -> u32 {
        0
    }
    /// The Act II lair warp check's answer, published by the quest control
    /// once per tick. Default: nothing.
    fn set_lair_open(&mut self, open: bool) {}
    /// The Arreat Summit warp check's answer (leaving the summit for 118 or
    /// 128), published by the quest control once per tick. Default: nothing.
    fn set_summit_open(&mut self, open: bool) {}
    /// The Ancients' gate answer source: the fight's armed byte, published
    /// by the quest control once per tick. Default: nothing.
    fn set_ancients_armed(&mut self, armed: bool) {}
    /// The not-intro test for `chain`: no record with the chain gives true,
    /// else the record's not-intro byte. The quest control publishes its
    /// records' answers once per tick. Default: false (no quest control).
    fn quest_not_intro(&self, chain: u8) -> bool {
        false
    }
    /// The quest control's not-intro bytes `(chain, not_intro)` of every
    /// record (for [`Self::quest_not_intro`]). Default: nothing.
    fn publish_not_intro(&mut self, records: &[(u8, bool)]) {}
    /// A quest event for the quest control (`take_quest_events`).
    /// Default: dropped.
    fn queue_quest_event(&mut self, e: QuestEvent) {}
    /// A caged barbarian's group state by GUID: (the group counter is
    /// non-zero in a not-intro record, the group's portal GUID when it is
    /// spawned). Default: (false, none).
    fn quest_rescue(&self, guid: u32) -> (bool, Option<u32>) {
        (false, None)
    }
    /// The quest control's caged-barbarian states (GUID, counting,
    /// portal) of every group, once per tick. Default: nothing.
    fn publish_rescue(&mut self, barbarians: &[(u32, bool, Option<u32>)]) {}
    /// Tyrael's spawn from a missile body. Default: nothing.
    fn missile_spawn_tyrael(&mut self, room: Option<RoomId>, missile: UnitId, x: i32, y: i32) {}
    /// The Durance of Hate warp check's answer (into level 100 from
    /// anywhere but level 101), published by the quest control once per
    /// tick. Default: nothing.
    fn set_durance_open(&mut self, open: bool) {}
    /// The Act III answers the town NPCs' map AI reads (Alkor and Ormus),
    /// published by the quest control once per tick. Default: nothing.
    fn set_act3_npc_answers(&mut self, alkor_bird: bool, ormus_altar: Option<(i32, i32)>) {}
    /// The player's hireling (the wired host answers it from the hireling
    /// list). Default: none.
    fn player_hireling(&self, player: UnitId) -> Option<UnitId> {
        None
    }
    /// Clears the player's busy field (C->S 0x48; read by
    /// [`Pending::object_player_busy`]). Default: nothing.
    fn clear_player_busy(&mut self, player: UnitId) {}
    /// Clears NPC `class`'s intro bit in the player's record for
    /// `difficulty` (C->S 0x4D; which of the two bit fields is cleared is
    /// not known). Default: nothing.
    fn clear_npc_intro(&mut self, player: UnitId, difficulty: u8, class: u16) {}
    /// The weapon switch of C->S 0x60: (result, fail). `None` (the
    /// default): no provider, the id stays a stub.
    fn weapon_switch(&mut self, game: &mut Game, player: UnitId) -> Option<(u32, bool)> {
        None
    }
}

/// Every default.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoPending;

impl Pending for NoPending {}
