//! What AI code needs from the rest of the game. Each trait names the
//! system that answers it; tests use small fakes. The decisions (draws,
//! tests, delays) stay in the AI modules.
//!
//! [`AiHost`] is the whole set. It is object-safe, so a mod game's AI
//! bodies (`docs/plans/mods/seams.md` MS5) run on `dyn AiHost` with the
//! same helpers D2RUST's bodies use.

use crate::game::Game;
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};
use crate::world::quests::act2::q4::JerhynStep;

/// Where a mode request points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeTarget {
    Unit(UnitId),
    Point(i32, i32),
}

/// Monster and unit fields. Provider: the unit, stat and stat-list
/// systems, with monster data from monster init
/// ([`crate::monsters::init`]).
pub trait AiUnits {
    /// The unit seed (unit +0x20).
    fn seed(&mut self, unit: UnitId) -> &mut Seed;
    /// Unit class (monstats row for monsters).
    fn class(&self, unit: UnitId) -> i32;
    /// Animation mode (unit +0x10).
    fn anim_mode(&self, unit: UnitId) -> u8;
    fn has_state(&self, unit: UnitId, state: u16) -> bool;
    /// The rules the game runs under: a mod game's AI table rows and
    /// bodies come from them. Default: D2RUST's own.
    fn rules(&self) -> &crate::rules::Ruleset {
        &crate::rules::ORIGINAL
    }
    /// Clears the uninterruptable state without its timer part: state 54 (and
    /// state 92 for players). The AI code cancels the type-2 events.
    fn clear_uninterruptable(&mut self, game: &mut Game, unit: UnitId);
    /// The unit is dead.
    fn is_dead(&self, unit: UnitId) -> bool;
    /// Position in subtiles.
    fn position(&self, unit: UnitId) -> (i32, i32);
    /// Unit size.
    fn size(&self, unit: UnitId) -> i32;
    /// Act (unit +0x18).
    fn act(&self, unit: UnitId) -> u8;
    /// `levels.txt` row of the unit's level (the level of its room in
    /// the game's lists).
    fn level_id(&self, game: &Game, unit: UnitId) -> i32;
    /// Monster level (the teleport heal reads it).
    fn monster_level(&self, unit: UnitId) -> i32;
    /// Life in percent of max life.
    fn life_percent(&self, unit: UnitId) -> i32;
    /// Adds life (fixed point), capped at max life.
    fn add_life(&mut self, unit: UnitId, amount: i32);
    /// Monster data `dwAiState` (read by the AI's state test).
    fn ai_state(&self, unit: UnitId) -> u32;
    /// The unit's source unit (set by `link_source`), none
    /// when unit +0xC8 lacks bit 0x400.
    fn source_unit(&self, _unit: UnitId) -> Option<UnitId> {
        None
    }
    /// Alignment: 0 evil, 1 neutral, 2 good.
    fn alignment(&self, unit: UnitId) -> u8;
    /// Monster type flag 8.
    fn is_unique(&self, unit: UnitId) -> bool;
    /// Champion type flag.
    fn is_champion(&self, unit: UnitId) -> bool;
    fn is_boss(&self, unit: UnitId) -> bool;
    /// Monster vision record (monster data +0x50): its +0x24 field, or
    /// `None` without a record.
    fn vision_seen(&self, unit: UnitId) -> Option<u32>;
    /// The target-seen update of the vision record.
    fn mark_seen(&mut self, unit: UnitId, value: u32);
    /// Type-10 handler: clears monster data +0x34, +0x38.
    fn ai_reset(&mut self, unit: UnitId);
    /// Interacting with a player.
    fn interacting(&self, unit: UnitId) -> bool;
    /// Player busy.
    fn busy(&self, unit: UnitId) -> bool;
    /// Monster data +0x30: the NPC interaction block exists
    /// (the NPC interaction handler reads it).
    fn has_interaction_block(&self, unit: UnitId) -> bool;
    /// `player` is in the NPC's interaction list.
    fn in_interaction_list(&self, npc: UnitId, player: UnitId) -> bool;
    /// Sets stat 6 (life).
    fn set_life(&mut self, unit: UnitId, value: i32);
    /// A stat of the unit, layer 0 (Baboon reads stat 74).
    fn stat(&self, unit: UnitId, stat: u16) -> i32;
    /// Unit set.
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32);
    /// Sets bits of the unit flags (unit +0xC4; FoulCrowNest's 0x20000).
    fn set_unit_flag(&mut self, unit: UnitId, mask: u32);
    /// State on / off (SandRaider's states 90, 91): a state outside
    /// 0 … count − 1
    /// does nothing; else the toggle, then the unit's update-queue insert
    /// whether or not the bit changed.
    fn set_state(&mut self, game: &mut Game, unit: UnitId, state: u16, on: bool);
    /// The unit's path target unit (`None` when there is
    /// none).
    fn path_target(&self, unit: UnitId) -> Option<UnitId>;
}

/// The mode machinery, paths and sounds. Provider: the unit system
/// (unit modes and movement).
pub trait AiModes {
    /// Requests a mode change; false when the mode start failed (which
    /// then falls into the neutral start, [`super::neutral_mode_start`]).
    fn change_mode(&mut self, game: &mut Game, unit: UnitId, mode: u8, target: ModeTarget) -> bool;
    /// [`Self::change_mode`] with the request record's path-type byte
    /// (+0x15) overwritten with `path_byte` after the builder set
    /// it (100 = no path, 101 = type 13, else the path type). Default:
    /// the plain request.
    fn change_mode_path_byte(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        mode: u8,
        target: ModeTarget,
        path_byte: u8,
    ) -> bool {
        let _ = path_byte;
        self.change_mode(game, unit, mode, target)
    }
    /// [`AiModes::change_mode`] (or, with a path byte,
    /// [`AiModes::change_mode_path_byte`]) with the monster's pending
    /// velocity request, which the mode set's movement
    /// set-up consumes for every mode but GH: a provider
    /// that runs the set-up takes it and leaves `velocity` zeroed.
    /// Default: the plain change, the request kept.
    fn change_mode_with(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        mode: u8,
        target: ModeTarget,
        path_byte: Option<u8>,
        velocity: &mut super::VelocityRequest,
    ) -> bool {
        let _ = velocity;
        match path_byte {
            Some(b) => self.change_mode_path_byte(game, unit, mode, target, b),
            None => self.change_mode(game, unit, mode, target),
        }
    }
    /// The mode written directly (the inline thinks use it): the unit is
    /// queued and gets flag 0x1; no mode start.
    /// mode start.
    fn set_anim_mode(&mut self, game: &mut Game, unit: UnitId, mode: u8);
    /// The path step count.
    fn set_path_steps(&mut self, unit: UnitId, steps: i32);
    /// Path flag 0x800 (blocked step).
    fn path_blocked(&self, unit: UnitId) -> bool;
    /// Stops the unit's path (NPC interaction).
    fn stop_path(&mut self, unit: UnitId);
    /// Current skill := `skill`; false when the id is out of range.
    fn set_current_skill(&mut self, unit: UnitId, skill: i32) -> bool;
    /// The mode request builder clears the current skill: a plain mode
    /// request has no used skill.
    fn clear_current_skill(&mut self, _unit: UnitId) {}
    /// Unit flag 0x40 (set by the skill-at-unit request).
    fn set_skill_flag(&mut self, unit: UnitId);
    /// The class has the mode.
    fn class_has_mode(&self, class: i32, mode: u8) -> bool;
    /// Monster sound, optionally to one player.
    fn play_sound(&mut self, game: &mut Game, unit: UnitId, sound: u32, to: Option<UnitId>);
    /// The gethit branch: mode change to gethit (3) with last-hit
    /// class 160.
    fn knockback_to_gethit(&mut self, game: &mut Game, unit: UnitId);
    /// Walk in radius: the provider computes the destination
    /// and walks, consuming the
    /// staged velocity request as [`AiModes::change_mode_with`] does.
    fn walk_in_radius(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        target: UnitId,
        a: i32,
        b: i32,
        velocity: &mut super::VelocityRequest,
    ) -> bool;
    /// Operates a door (the object system's door operation).
    fn operate_door(&mut self, game: &mut Game, unit: UnitId, door: UnitId);
    /// Starts an overlay (stat 178
    /// `unit_dooverlay`).
    fn start_overlay(&mut self, unit: UnitId, overlay: i32);
    /// Sets the path facing.
    fn set_facing(&mut self, unit: UnitId, dir: i32);
}

/// Rooms, collision and line tests. Provider: the DRLG (and
/// the unit system for the unit-to-unit tests).
pub trait AiWorld {
    fn in_town(&self, game: &Game, room: RoomId) -> bool;
    /// The room's "LOS draw" test.
    fn los_draw(&self, game: &Game, room: RoomId) -> bool;
    /// The unit's position collides with `mask`.
    fn collides(&self, game: &Game, unit: UnitId, mask: u16) -> bool;
    /// Blocked line between a and b.
    fn line_blocked(&self, game: &Game, a: UnitId, b: UnitId) -> bool;
    /// Melee range.
    fn in_melee_range(&self, game: &Game, a: UnitId, b: UnitId) -> bool;
    /// Whether the target can be reached directly.
    fn can_reach_directly(&self, game: &Game, unit: UnitId, target: UnitId) -> bool;
    /// A free spot for a teleport; its own draws on the room
    /// seed. Returns the spot and its room.
    fn find_spot(&mut self, game: &mut Game, unit: UnitId) -> Option<(i32, i32, RoomId)>;
    /// The four last-dead GUIDs of a room (room +0x38..+0x44), as units.
    fn last_dead(&self, game: &Game, room: RoomId) -> [Option<UnitId>; 4];
    /// The monster footprint test.
    fn footprint_ok(&self, game: &Game, class: i32, room: Option<RoomId>, x: i32, y: i32) -> bool;
}

/// Target sources: the game's target-node lists, room scans and forced
/// targets. Provider: the units wiring (target lists) and the skills
/// wiring (forced targets).
pub trait AiTargets {
    /// The game's target-node lists: each player list (a player head and
    /// its attached units), then the good NPCs and the bone walls.
    fn target_nodes(&self, game: &Game) -> crate::game::TargetLists;
    /// The forced target (line of sight `a`, size `s`) and its full-size
    /// distance.
    fn forced_target(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        a: bool,
        s: bool,
        ai: &super::AiStore,
    ) -> Option<(UnitId, i32)>;
    /// Scan 5 within 35 with LOS flag `los`, then
    /// the good-target filter; target and distance.
    fn good_target_search(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        los: bool,
        ai: &super::AiStore,
    ) -> Option<(UnitId, i32)>;
    /// The pick between `main` and the slot-9
    /// alternative, each with its distance: the alternative, main, or
    /// the unit scan 7 put in main's place.
    fn choose_alternative(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        main: Option<(UnitId, i32)>,
        alt: (UnitId, i32),
        ai: &super::AiStore,
    ) -> Option<(UnitId, i32)>;
    /// Secondary target, distance (0x7FFFFFFF if none) and
    /// melee flag.
    fn secondary_target(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        ai: &super::AiStore,
    ) -> (Option<UnitId>, i32, bool);
    /// Nearest interacting player within 15 and the "close"
    /// flag (distance < 4); the NPC itself when none.
    fn nearest_player(&mut self, game: &mut Game, unit: UnitId) -> (UnitId, bool);
    /// Scan 8 (start distance 9): a door.
    fn find_door(&mut self, game: &mut Game, unit: UnitId) -> Option<UnitId>;
    /// objects.txt `MonsterOK` of the door.
    fn door_monster_ok(&self, door: UnitId) -> bool;
    /// Scan 11: alternative target for the special walk, with distance.
    fn special_walk_target(&mut self, game: &mut Game, unit: UnitId) -> Option<(UnitId, i32)>;
    /// SandRaider's help search: scan 1 for the nearest
    /// other monster with alignment 0 not in mode 0 or 12, by squared
    /// distance (strictly smaller wins).
    fn nearest_evil_monster(&mut self, game: &mut Game, unit: UnitId) -> Option<UnitId>;
}

/// The quest NPC whose portal functions NpcOutOfTown calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalNpc {
    /// cain1: Act 1 quest 4 functions.
    Cain,
    /// drehyaiced: Act 5 quest 3 functions.
    Drehya,
}

/// Quest calls of the AI functions. Provider: the quest system
/// ([`crate::world::quests`]).
pub trait AiQuests {
    /// Set up the portal coordinates of the town portal NPC; false =
    /// failed.
    fn portal_setup(&mut self, game: &mut Game, unit: UnitId, npc: PortalNpc) -> bool;
    /// Spawn the town portal.
    fn spawn_town_portal(&mut self, game: &mut Game, unit: UnitId, npc: PortalNpc);
    /// Spawn the portal out of town; false
    /// = failed.
    fn spawn_outside_portal(&mut self, game: &mut Game, unit: UnitId, npc: PortalNpc) -> bool;
    /// The portal coordinates; `None` =
    /// none.
    fn portal_coords(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        npc: PortalNpc,
    ) -> Option<(i32, i32)>;
    /// drehyaiced's per-think quest update.
    fn drehya_update(&mut self, game: &mut Game);
    /// drehyaiced's walk gate (nonzero → wait).
    fn drehya_wait(&mut self, game: &mut Game) -> bool;

    // ---- the Npc class cases ----

    /// jerhyn: `ACT2Q4_IsJerhynPalaceActivated`.
    fn jerhyn_palace_active(&mut self, game: &mut Game) -> bool;
    /// jerhyn: `ACT2Q4_GetAndUpdatePalaceNpcState`: (a, b)
    /// and the walk or placement it makes on the unit.
    fn jerhyn_npc_state(&mut self, game: &mut Game, unit: UnitId) -> JerhynStep;
    /// jerhyn: the placement of [`JerhynStep::PlaceAt`] succeeded (+0x0F,
    /// +0x14 := 1).
    fn jerhyn_placed(&mut self, game: &mut Game);
    /// jerhyn: `ACT2Q4_IsGuardMoving`.
    fn guard_moving(&mut self, game: &mut Game, unit: UnitId) -> bool;
    /// alkor: `ACT3Q4_GoldenBirdBroughtToAlkor`.
    fn alkor_bird(&mut self, game: &mut Game) -> bool;
    /// alkor: `ACT3Q4_ResetAlkor`.
    fn alkor_reset(&mut self, game: &mut Game);
    /// ormus: `ACT3Q3_GetAltarCoordinates`.
    fn ormus_altar(&mut self, game: &mut Game) -> Option<(i32, i32)>;
    /// ormus: `ACT3Q3_SetAltarMode`.
    fn ormus_set_altar_mode(&mut self, game: &mut Game);
    /// cain5: `ACT1Q4_GetCainPortalInTownCoordinates`.
    fn cain_town_coords(&mut self, game: &mut Game, unit: UnitId) -> Option<(i32, i32)>;
    /// cain5: `ACT1Q4_OnCainInTownActivated`.
    fn cain_in_town_activated(&mut self, game: &mut Game, unit: UnitId);
    /// drehya: `ACT5Q4_AnyaOpenPortal`.
    fn anya_open_portal(&mut self, game: &mut Game, unit: UnitId);
}

/// Quest calls of the Act II–V bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestCall {
    /// `ACT2Q1_OnRadamentActivated`.
    RadamentActivated,
    /// `ACT2Q5_OnSummonerActivated`.
    SummonerActivated,
    /// `ACT4Q1_OnIzualActivated`.
    IzualActivated,
    /// Shenk's quest call.
    Shenk,
    /// Nihlathak's quest call.
    Nihlathak,
    /// `ACT5Q5_IsNotActivatable`: the result.
    AncientsNotActivatable,
    /// `ACT5Q5_IsActivatable`: the result.
    AncientsActivatable,
    /// The Ancients' portal exists.
    AncientsPortal,
    /// Baal enters the Worldstone Chamber.
    BaalToStairs,
}

/// Seams of the Act II–V bodies.
/// Providers: the unit and stat systems (unit fields, stat lists), the
/// skill system (skill entries and checks), path placement
/// (patterns, placement, collision), population and monster init
/// (spawns), damage (kill) and the quests.
pub trait AiActs {
    // ---- unit fields ----

    /// Unit flags (unit +0xC4).
    fn unit_flags(&self, unit: UnitId) -> u32;
    /// Clears bits of the unit flags (Vulture take-off).
    fn clear_unit_flag(&mut self, unit: UnitId, mask: u32);
    /// Max life (stat 7, fixed point).
    fn max_life(&self, unit: UnitId) -> i32;
    /// Max mana.
    fn max_mana(&self, unit: UnitId) -> i32;
    /// A state of flag group `g` (2 `hide`, 33 `udead`).
    fn has_state_group(&self, unit: UnitId, g: u8) -> bool;
    /// States count (data tables +0xC4).
    fn states_count(&self) -> i32;
    /// An active stat list with list flags
    /// (0x20: a curse).
    fn has_list_flag(&self, unit: UnitId, flags: u32) -> bool;
    /// B is hostile to a.
    fn hostile(&self, game: &Game, a: UnitId, b: UnitId) -> bool;
    /// The unit's owner unit.
    fn owner(&self, game: &Game, unit: UnitId) -> Option<UnitId>;
    /// A monster's owner record (type, GUID).
    fn owner_record(&self, unit: UnitId) -> Option<(i32, u32)>;
    /// The quest flag test on the player's quest record of
    /// `difficulty` (player data +0x10 + 4 × difficulty).
    fn quest_flag(&self, player: UnitId, difficulty: u8, quest: i32, flag: i32) -> bool;
    /// The player's portal GUID.
    fn portal_guid(&self, player: UnitId) -> Option<u32>;
    /// Monster component byte at index `i`.
    fn component(&self, unit: UnitId, i: usize) -> u8;
    /// The unit's target unit.
    fn target_unit(&self, game: &Game, unit: UnitId) -> Option<UnitId>;
    /// The target override.
    fn set_target_override(&mut self, unit: UnitId, kind: i32, guid: u32);
    /// The monstats chain byte +0x4B.
    fn chain_index(&self, class: i32) -> i32;
    /// The class of `class`'s chain for the
    /// room's level.
    fn class_for_level(&self, game: &Game, room: Option<RoomId>, class: i32) -> i32;

    // ---- skills ----

    /// The level with bonus of the unit's
    /// skill entry: `highest` = the highest entry, else the
    /// entry with owner −1; `None` without an entry.
    fn skill_level(&self, unit: UnitId, skill: i32, highest: bool) -> Option<i32>;
    /// The unit's highest entry of `skill`: its skill id
    /// and mode (entry +0x08).
    fn skill_entry(&self, unit: UnitId, skill: i32) -> Option<(i32, u8)>;
    /// The left or right skill: id and level
    /// with bonus.
    fn hand_skill(&self, unit: UnitId, right: bool) -> Option<(i32, i32)>;
    /// Adds the
    /// skill and make it the right skill (Duriel's aura).
    fn add_right_skill(&mut self, game: &mut Game, unit: UnitId, skill: i32, level: i32);
    /// Assign the skill at `level`.
    fn assign_skill(&mut self, game: &mut Game, unit: UnitId, skill: i32, level: i32);
    /// The param of the unit's highest `skill` entry; false
    /// when there is no entry.
    fn set_skill_param(&mut self, unit: UnitId, skill: i32, value: i32) -> bool;
    /// The skill check.
    fn skill_check(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        skill: i32,
        target: Option<UnitId>,
        x: i32,
        y: i32,
    ) -> bool;
    /// The corpse search of
    /// Corpse Explosion (the skills module).
    fn corpse_search(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        target: Option<UnitId>,
        skill: i32,
        level: i32,
    ) -> Option<UnitId>;

    // ---- path, placement, collision ----

    /// Path pattern (path +0x48).
    fn path_pattern(&self, unit: UnitId) -> i32;
    fn set_path_pattern(&mut self, unit: UnitId, pattern: i32);
    /// Move mask (path +0x50).
    fn set_move_mask(&mut self, unit: UnitId, mask: u16);
    /// Places the unit at (x, y) of the room (not exact, no
    /// alternative spot); false = failed.
    fn place_unit(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        room: Option<RoomId>,
        x: i32,
        y: i32,
    ) -> bool;
    /// Pattern stamp.
    fn stamp_pattern(
        &mut self,
        game: &mut Game,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        pattern: i32,
        mask: u16,
    );
    /// Clears `bits` of the one cell at (x, y).
    fn clear_cell(&mut self, game: &mut Game, room: Option<RoomId>, x: i32, y: i32, bits: u16);
    /// The point collides.
    fn point_collides(&self, game: &Game, room: Option<RoomId>, x: i32, y: i32, mask: u16) -> bool;
    fn pattern_collides(&self, game: &Game, unit: UnitId, pattern: i32, mask: u16) -> bool;
    /// A free point near (x, y).
    fn free_point(
        &mut self,
        game: &mut Game,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        size: i32,
    ) -> Option<(i32, i32)>;
    /// A free spot for the class
    /// near (x, y) (it draws from the game seed).
    fn free_spot_for(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        class: i32,
        x: i32,
        y: i32,
    ) -> Option<(i32, i32)>;
    /// The room at (x, y) from the unit's room.
    fn room_at(&self, game: &Game, unit: UnitId, x: i32, y: i32) -> Option<RoomId>;
    /// The walk-in-radius core with a
    /// mode, using the provider's movement geometry.
    fn move_in_radius(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        target: UnitId,
        mode: u8,
        a: i32,
        b: i32,
    ) -> bool;
    /// The path target unit.
    fn set_path_target(&mut self, unit: UnitId, target: UnitId);
    /// Boss pick slot 9: path target :=
    /// target, path type 2, compute the path; whether it has points.
    fn path_has_points(&mut self, game: &mut Game, unit: UnitId, target: UnitId) -> bool;
    /// The 64-step direction from the unit to the target.
    fn direction64(&self, unit: UnitId, target: UnitId) -> i32;
    /// Stops the path (BaalToStairs).
    fn stop_unit_path(&mut self, unit: UnitId);

    // ---- spawns, removal, damage, stat lists ----

    #[allow(clippy::too_many_arguments)]
    fn spawn_monster(
        &mut self,
        game: &mut Game,
        room: RoomId,
        x: i32,
        y: i32,
        class: i32,
        mode: u8,
        spread: i32,
        flags: u32,
    ) -> Option<UnitId>;
    /// Lends the AI store to the host for the duration of a spawn, so the
    /// created unit's control block is allocated and its AI installed
    /// inside the creation (returned by
    /// [`Self::return_ai_store`]). A host without a creation path hands the
    /// store straight back.
    fn lend_ai_store(&mut self, store: super::AiStore) -> Option<super::AiStore> {
        Some(store)
    }
    /// Takes the AI store back after a spawn.
    fn return_ai_store(&mut self) -> Option<super::AiStore> {
        None
    }
    fn kill(&mut self, game: &mut Game, unit: UnitId, killer: Option<UnitId>);
    /// The unit leaves its room and is removed (room, unit list,
    /// path and collision).
    fn remove_unit(&mut self, game: &mut Game, unit: UnitId);
    /// Baal clone links: the
    /// clone joins the unit's minions and each stores the other as owner.
    fn link_clone(&mut self, game: &mut Game, unit: UnitId, clone: UnitId);
    /// Reinitialize as `class`.
    fn reinit_class(&mut self, game: &mut Game, unit: UnitId, class: i32, mode: u8);
    /// BaalThrone: a stat list (flags 0, no expiry, owned by
    /// the unit) attached, its state := 142, stat 355 := `class`.
    fn change_class_list(&mut self, game: &mut Game, unit: UnitId, class: i32);
    /// WillOWisp: the magic-find stat list on `target`
    /// (stat 80 := `value`, expiry and type-12 timer at `expire`).
    fn wisp_buff(&mut self, game: &mut Game, target: UnitId, value: i32, expire: i32);
    /// S→C 0xA4 (client preload) with `class` on the unit.
    fn preload_class(&mut self, game: &mut Game, unit: UnitId, class: i32);
    /// The unit find of WillOWisp (size 32,
    /// flags 0x583; open question 3): the units it visits, in its order.
    fn wisp_find(&mut self, game: &mut Game, unit: UnitId) -> Vec<UnitId>;
    /// BaalThrone wave `w` (superunique 61 + w, mapped through the
    /// superunique table): (mapped superunique id, class);
    /// `None` without a record.
    fn wave(&self, w: i32) -> Option<(i32, i32)>;
    /// Clears the room's portal flag.
    fn clear_room_portal_flag(&mut self, game: &mut Game, room: Option<RoomId>);
    /// A quest call; the result where the call returns one.
    fn quest_call(&mut self, game: &mut Game, unit: UnitId, call: QuestCall) -> bool;
}

/// One hireling.txt row as the hireling skill pick reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HireRow {
    /// `Level`.
    pub level: i32,
    /// `DefaultChance` (+0x64).
    pub default_chance: i32,
    /// `Skill1`..`Skill6` (+0x78 + 4i).
    pub skill: [i32; 6],
    /// `Chance1`..`Chance6` (+0x90 + 4i).
    pub chance: [i32; 6],
    /// `ChancePerLvl1`..`ChancePerLvl6` (+0xA8 + 4i).
    pub chance_per_lvl: [i32; 6],
    /// `Mode1`..`Mode6` (+0xC0 + i).
    pub mode: [u8; 6],
}

/// Quest seams of the one-row AIs (pets, traps, quest NPCs of
/// Acts II–V).
/// The result is the call's return value where it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestHook {
    /// Tyrael (class 251) may leave.
    TyraelLeave,
    /// Tyrael left.
    TyraelGone,
    /// Izual's ghost (class 406) may leave.
    IzualGhostLeave,
    /// Izual's ghost left.
    IzualGhostGone,
    /// The palace door is open (JarJar).
    PalaceDoorOpen,
    /// The palace guard steps aside (JarJar).
    PalaceGuardAside,
    /// The Dark Wanderer's minion hook.
    DarkWandererGone,
    /// A caged barbarian leaves (Wussie).
    WussieLeaving,
    /// The barbarian left.
    WussieLeave,
    /// P can rescue the barbarian.
    WussieCanRescue,
    /// Rescue.
    WussieRescue,
    /// The barbarian waits for P.
    WussieWait,
}

/// Seams of the one-row AI bodies (pets,
/// hirelings, summons, traps, spawners, quest NPCs). Every method has a
/// narrow default (nothing found, nothing done) until its owner wires
/// it; the owners are named per method.
pub trait AiSummons {
    // ---- paths ----

    /// The path's final point (path +0x18 / +0x1A): where the unit is
    /// heading. Default: (0, 0).
    fn path_final_point(&self, _unit: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// The path's target point (path +0x10 / +0x12). Default: (0, 0).
    fn path_target_point(&self, _unit: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// Boss pick slot 9 with no best: path
    /// target unit := 0 (type and GUID kept), path type 2, the path
    /// computed toward its stored target point; whether it has points.
    /// Default: none.
    fn path_has_points_no_target(&mut self, _game: &mut Game, _unit: UnitId) -> bool {
        false
    }
    /// The path target point := (x, y).
    fn set_path_target_point(&mut self, _unit: UnitId, _x: i32, _y: i32) {}
    /// The 64-step direction from the unit's
    /// position to (x, y). Default: 0.
    fn direction64_to(&self, _unit: UnitId, _x: i32, _y: i32) -> i32 {
        0
    }
    /// Snaps the path direction. Default: none.
    fn snap_direction(&mut self, _unit: UnitId, _dir: i32) {}
    /// The line a→b is blocked for `mask`
    /// Default: clear.
    fn line_blocked_mask(&self, _game: &Game, _a: UnitId, _b: UnitId, _mask: u16) -> bool {
        false
    }

    // ---- rooms and placement (DRLG, path placement, population) ----

    /// The coordinate index at (x, y). Default: 0.
    fn coord_index(&self, _game: &Game, _room: Option<RoomId>, _x: i32, _y: i32) -> i32 {
        0
    }
    /// The spot class argument of the free spot
    /// search. Default: 0.
    fn spot_class(&self, _game: &Game, _room: Option<RoomId>, _x: i32, _y: i32) -> i32 {
        0
    }
    /// A free spot (it draws from the room seed). Default: none.
    fn free_spot(
        &mut self,
        _game: &mut Game,
        _room: Option<RoomId>,
        _cl: i32,
        _class: i32,
    ) -> Option<(i32, i32)> {
        None
    }
    /// A free point near (x, y).
    /// Default: none.
    #[allow(clippy::too_many_arguments)]
    fn free_point_masked(
        &mut self,
        _game: &mut Game,
        _room: Option<RoomId>,
        _x: i32,
        _y: i32,
        _size: i32,
        _mask: u16,
        _n: i32,
    ) -> Option<(i32, i32)> {
        None
    }
    /// The room's sub-tile box (x, y, w, h). Default: none.
    fn room_box(&self, _game: &Game, _room: RoomId) -> Option<(i32, i32, i32, i32)> {
        None
    }
    /// A dead monster of `class` in the
    /// room at (x, y) (or an inactive record with the dead bit). Default:
    /// false.
    fn class_dead_at(&self, _game: &Game, _room: RoomId, _x: i32, _y: i32, _class: i32) -> bool {
        false
    }
    /// The unit's target-node slot (unit +0xD0); 11 = none. Default: 11.
    fn target_slot(&self, _unit: UnitId) -> i32 {
        11
    }
    /// A node pushed at the head of the
    /// game's target-node list `slot`; unit +0xD0 := slot.
    fn register_target_node(&mut self, _game: &mut Game, _unit: UnitId, _slot: i32) {}
    /// Unit flags 2 (unit +0xC8) |= `mask`.
    fn set_unit_flags2(&mut self, _unit: UnitId, _mask: u32) {}
    /// Notes the AI control's spawn class (+0x3C) where a skill start run
    /// inside the think can read it. Default: nothing.
    fn note_spawn_class(&mut self, _unit: UnitId, _class: i32) {}
    /// The trap kind cached in the unit's level's monster region
    /// (+0x2D4); −1 = unset. Default: −1.
    fn trap_kind(&self, _game: &Game, _unit: UnitId) -> i32 {
        -1
    }
    /// Stores the trap kind in the unit's level's monster region.
    fn set_trap_kind(&mut self, _game: &mut Game, _unit: UnitId, _kind: i32) {}
    /// The monster classes of the unit's level's monster region (+0x14 +
    /// 0x34·i, count +0x10). Default: none.
    fn region_classes(&self, _game: &Game, _unit: UnitId) -> Vec<i32> {
        Vec::new()
    }

    // ---- players and pets ----

    /// Player data +0xA0 (next index) and +0xA8 (the 20 position history
    /// entries the player walked through). Default: empty.
    fn position_history(&self, _player: UnitId) -> (usize, [(i32, i32); 20]) {
        (0, [(0, 0); 20])
    }
    /// Player data +0x148 / +0x14C: the last placed point. Default: (0, 0).
    fn last_placed_point(&self, _player: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// The living units of the owner's pet lists (types 1 …
    /// count − 1, nodes without flag bit 0), in list order. Default: none.
    fn pets(&self, _game: &Game, _owner: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    /// The owner's total pet count. Default: 0.
    fn pet_count(&self, _owner: UnitId) -> i32 {
        0
    }
    /// The unit's pet type in the owner's lists; −1 = none.
    fn pet_type_of(&self, _game: &Game, _owner: UnitId, _unit: UnitId) -> i32 {
        -1
    }
    /// The `pettype.txt` row count (data tables +0xBF0;
    /// the pet type test reads it). Default: 0.
    fn pettype_count(&self) -> i32 {
        0
    }
    /// The hireling `Id` (+8 of the record) of the unit's
    /// node in the owner's pet lists. Default: none.
    fn hireling_id(&self, _game: &Game, _owner: UnitId, _unit: UnitId) -> Option<i32> {
        None
    }
    /// The hireling row. Default: none.
    fn hireling_row(&self, _game: &Game, _id: i32, _level: i32) -> Option<HireRow> {
        None
    }
    /// The unit's attack rating. Default: 0.
    fn attack_rating(&self, _unit: UnitId) -> i32 {
        0
    }
    /// The weapon mastery
    /// to-hit of the unit's weapon; `None` without a weapon.
    fn mastery_tohit(&self, _unit: UnitId) -> Option<i32> {
        None
    }

    // ---- skills ----

    /// A skills calc column
    /// (skill calc expressions). Default: 0.
    fn skill_calc(
        &mut self,
        _game: &mut Game,
        _unit: UnitId,
        _skill: i32,
        _calc: u32,
        _level: i32,
    ) -> i32 {
        0
    }
    /// The mode of the unit's entry of `skill` with owner −1
    /// (entry +0x08). Default: none.
    fn entry_mode(&self, _unit: UnitId, _skill: i32) -> Option<u8> {
        None
    }
    /// The base level of the unit's entry
    /// of `skill`. Default: none.
    fn skill_base_level(&self, _unit: UnitId, _skill: i32) -> Option<i32> {
        None
    }
    /// The unit's skill list, in list order:
    /// skill id and level with bonus. Default: empty.
    fn unit_skills(&self, _unit: UnitId) -> Vec<(i32, i32)> {
        Vec::new()
    }
    /// The unit has a skill list (unit +0xA8). Default: false.
    fn has_skill_list(&self, _unit: UnitId) -> bool {
        false
    }
    /// The class skill list of a player class (data tables +0xBA4 /
    /// +0xBAC). Default: empty.
    fn class_skills(&self, _class: i32) -> Vec<i32> {
        Vec::new()
    }
    /// Make `skill` the right skill.
    fn make_right_skill(&mut self, _game: &mut Game, _unit: UnitId, _skill: i32) {}
    /// The hand skill (left or right) := the
    /// unit's entry of `skill`.
    fn set_hand_skill(&mut self, _unit: UnitId, _skill: i32, _right: bool) {}
    /// The "both" range (3) reads as ranged (2) when
    /// set. Default: false.
    fn ranged_both(&self, _unit: UnitId) -> bool {
        false
    }
    /// A skill
    /// missile from `owner`. Default: none.
    #[allow(clippy::too_many_arguments)]
    fn skill_missile(
        &mut self,
        _game: &mut Game,
        _owner: UnitId,
        _skill: i32,
        _level: i32,
        _missile: i32,
        _x: i32,
        _y: i32,
    ) -> Option<UnitId> {
        None
    }
    /// A stores b as its owner.
    fn link_owner(&mut self, _game: &mut Game, _a: UnitId, _b: UnitId) {}
    /// The corpse search (unit find flags
    /// 0x1002, size n around the unit; corpse test, hostile
    /// test): the first match. Default: none.
    fn corpse_find(&mut self, _game: &mut Game, _unit: UnitId, _n: i32) -> Option<UnitId> {
        None
    }

    // ---- states ----

    /// A state of the same `group` (states +0x1E) as
    /// `state` is active on the unit. Default: false.
    fn state_group_active(&self, _unit: UnitId, _state: i32) -> bool {
        false
    }
    /// The unit has a `pgsv` state (state flag bit 4).
    fn has_pgsv_state(&self, _unit: UnitId) -> bool {
        false
    }
    /// The value of `stat` in the stat list of the unit's `state`
    /// `None` without the list or the stat.
    fn state_stat(&self, _unit: UnitId, _state: i32, _stat: i32) -> Option<i32> {
        None
    }
    /// The monster type flags (8 unique, …). Default: 0.
    fn monster_type_flags(&self, _unit: UnitId) -> u32 {
        0
    }

    // ---- quests and messages ----

    /// A quest hook; the result where the call returns one. Default: 0.
    fn quest_hook(
        &mut self,
        _game: &mut Game,
        _unit: UnitId,
        _player: Option<UnitId>,
        _hook: QuestHook,
    ) -> bool {
        false
    }
    /// Nonzero and the (possibly moved) point.
    /// Default: (false, x, y).
    fn palace_guard_point(&mut self, _game: &mut Game, x: i32, y: i32) -> (bool, i32, i32) {
        (false, x, y)
    }
    /// The Dark Wanderer's walk target.
    fn dark_wanderer_target(&mut self, _game: &mut Game, _unit: UnitId) -> Option<(i32, i32)> {
        None
    }
    /// `None` = no rescue portal; else the
    /// portal unit P (`None` = 0).
    fn rescue_portal(&mut self, _game: &mut Game, _unit: UnitId) -> Option<Option<UnitId>> {
        None
    }
    /// The quest active test: true when an active function wants the
    /// player to talk to the NPC, after S→C 0x8A NpcWantsInteract {1, the
    /// unit's GUID} went to the player's client. Default: no quest control,
    /// false.
    fn npc_wants_interact(&mut self, _game: &mut Game, _player: UnitId, _unit: UnitId) -> bool {
        false
    }
}

/// Everything AI code needs.
pub trait AiHost:
    AiUnits + AiModes + AiWorld + AiTargets + AiQuests + AiActs + AiSummons + AsDynAi
{
}

impl<
        T: AiUnits + AiModes + AiWorld + AiTargets + AiQuests + AiActs + AiSummons + AsDynAi + ?Sized,
    > AiHost for T
{
}

/// Any AI host as `dyn AiHost`, so generic AI code can hand its world to
/// a mod game's bodies (MS5).
pub trait AsDynAi {
    fn as_dyn_ai(&mut self) -> &mut dyn AiHost;
}

impl<T: AiUnits + AiModes + AiWorld + AiTargets + AiQuests + AiActs + AiSummons> AsDynAi for T {
    fn as_dyn_ai(&mut self) -> &mut dyn AiHost {
        self
    }
}
