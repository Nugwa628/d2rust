//! Monster AI on the action wiring: [`View`] implements
//! [`crate::monsters::ai::AiHost`]. It answers seeds, class, mode, states
//! (including clearing the uninterruptable state), the dead test, act,
//! the level of the unit's room, life and life writes, mode changes, the
//! attack flag, the town test and collision grids, door operation and
//! `MonsterOK` on the object state, and the game's rules (a mod game's
//! AI, `docs/plans/mods/seams.md` MS5). Path, targets, skills, sounds and
//! the monster data the lent monster world ([`super::monsters`]) doesn't
//! answer go to [`Pending`].

use crate::game::Game;
use crate::monsters::ai::{
    AiActs, AiModes, AiQuests, AiSummons, AiTargets, AiUnits, AiWorld, HireRow, ModeTarget,
    PortalNpc, QuestCall,
};
use crate::rng::Seed;
use crate::stats::stat;
use crate::units::hooks::Sim;
use crate::units::record::flags;
use crate::units::{RoomId, UnitId, UnitType};

use super::objects::{CainPortalAnswer, CainPortalCall, ObjectRoute};
use super::units::clear_uninterruptable;
use super::{Pending, View, WiringError};
use crate::world::objects::Dispatch;
use crate::world::quests::act2::q4::JerhynStep;

/// Monster mode 3, get-hit.
const MODE_GETHIT: u32 = 3;

impl<X: Pending> View<'_, X> {
    /// A dead (mode 12) prison
    /// door (class 434) among the units of the rooms adjacent to `u`'s
    /// room (the room itself included).
    fn dead_prison_door_near(&self, game: &Game, u: UnitId) -> bool {
        use crate::path::collision::CollisionRooms;
        let Some(room) = game.lists.unit(u).and_then(|e| e.room()) else {
            return false;
        };
        let d = &self.h.drlg;
        (0..d.adjacent_count(room))
            .filter_map(|i| d.adjacent(room, i))
            .flat_map(|r| game.lists.room_units(r))
            .any(|m| {
                self.units.get(m).is_some_and(|r| {
                    r.ty == crate::units::UnitType::Monster && r.class == 434 && r.mode == 12
                })
            })
    }
    /// A monster's class and its monstats `interact` flag; `None` for a non-monster.
    fn npc_interact(&self, unit: UnitId) -> Option<(u16, bool)> {
        let r = self
            .units
            .get(unit)
            .filter(|r| r.ty == crate::units::UnitType::Monster)?;
        let class = u16::try_from(r.class).ok()?;
        let interact = self
            .h
            .tables
            .combat
            .monstats
            .get(usize::from(class))
            .is_some_and(|m| m.interact);
        Some((class, interact))
    }
}

impl<X: Pending> View<'_, X> {
    /// [`super::MonsterWorld::spawn_point`] in `unit`'s room on the lent
    /// monster world: `None` when no world (or no room) can run it.
    fn spawn_point(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        class: i32,
    ) -> Option<Option<(i32, i32)>> {
        let room = game.lists.unit(unit).and_then(|e| e.room())?;
        let mut sim = Sim {
            game: &mut *game,
            units: &mut *self.units,
            stats: &mut *self.stats,
            data: self.data,
        };
        self.h
            .with_monster_world(|w, h| w.spawn_point(&mut sim, h, room, class))
            .flatten()
    }
}

impl<X: Pending> AiUnits for View<'_, X> {
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.h.rules
    }
    fn seed(&mut self, unit: UnitId) -> &mut Seed {
        View::seed(self, unit)
    }
    fn class(&self, unit: UnitId) -> i32 {
        self.units.get(unit).map_or(-1, |r| r.class as i32)
    }
    fn anim_mode(&self, unit: UnitId) -> u8 {
        self.units.get(unit).map_or(0, |r| r.mode as u8)
    }
    fn has_state(&self, unit: UnitId, s: u16) -> bool {
        self.stats.has_state(unit, u32::from(s))
    }
    fn clear_uninterruptable(&mut self, game: &mut Game, unit: UnitId) {
        clear_uninterruptable(self, game, unit);
    }
    fn is_dead(&self, unit: UnitId) -> bool {
        self.units.is_dead(unit)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.h.path_position(unit)
    }
    fn size(&self, unit: UnitId) -> i32 {
        self.path_size(unit)
    }
    fn act(&self, unit: UnitId) -> u8 {
        self.units.get(unit).map_or(0, |r| r.act)
    }
    /// The level id of the unit's room (DRLG); 0 without a room.
    fn level_id(&self, game: &Game, unit: UnitId) -> i32 {
        game.lists
            .unit(unit)
            .and_then(|e| e.room())
            .and_then(|r| self.h.drlg.level_id(game, r))
            .map_or(0, |l| l as i32)
    }
    /// The monster level: stat 12 (`level`) of a
    /// monster with monster data in the lent monster world; else
    /// [`Pending::monster_level`].
    ///
    /// TODO: which getter the game uses for "level" is not known; the unit
    /// total (layer 0) is read.
    fn monster_level(&self, unit: UnitId) -> i32 {
        match self.h.monster_data(unit) {
            Some(_) => self.stats.unit_total(unit, stat::LEVEL, 0),
            None => self.h.x.monster_level(unit),
        }
    }
    /// Life in percent of max life.
    ///
    /// TODO: the rounding of "life %" is not known; this is
    /// `100 · life / max` truncating (0 with max 0).
    fn life_percent(&self, unit: UnitId) -> i32 {
        let life = i64::from(self.stats.unit_total(unit, stat::HITPOINTS, 0));
        let max = i64::from(self.stats.max_life(unit));
        if max == 0 {
            0
        } else {
            (life * 100 / max) as i32
        }
    }
    /// Life += amount (8.8), capped at max life.
    fn add_life(&mut self, unit: UnitId, amount: i32) {
        let life = self
            .stats
            .unit_total(unit, stat::HITPOINTS, 0)
            .wrapping_add(amount)
            .min(self.stats.max_life(unit));
        self.set_base(unit, stat::HITPOINTS, life);
    }
    fn ai_state(&self, unit: UnitId) -> u32 {
        match self.h.monster_data(unit) {
            Some(m) => m.ai_state,
            None => self.h.x.ai_state(unit),
        }
    }
    fn source_unit(&self, unit: UnitId) -> Option<UnitId> {
        if self.units.get(unit)?.flags2 & 0x400 == 0 {
            return None;
        }
        self.h.unit_source.get(&unit).copied()
    }
    /// Alignment, read live: a confused monster's 1 sends it to scan 5.
    fn alignment(&self, unit: UnitId) -> u8 {
        self.alignment_stat(unit)
    }
    /// Type flag 8 ([`super::ActionHooks::monster_flag`]).
    fn is_unique(&self, unit: UnitId) -> bool {
        self.h.monster_flag(unit, 8)
    }
    /// Type flag 4.
    fn is_champion(&self, unit: UnitId) -> bool {
        self.h.monster_flag(unit, 4)
    }
    /// A monster whose monstats flags have the `boss` bit (0x40).
    fn is_boss(&self, unit: UnitId) -> bool {
        let Some(r) = self.units.get(unit) else {
            return false;
        };
        r.ty == UnitType::Monster
            && usize::try_from(r.class)
                .ok()
                .and_then(|c| self.h.tables.combat.monstats.get(c))
                .is_some_and(|m| m.boss)
    }
    /// The monster's vision record (a coordinate record) and its second
    /// word: a monster of the lent monster
    /// world reads [`super::ActionHooks::vision_seen`]; other units ask
    /// [`Pending`].
    fn vision_seen(&self, unit: UnitId) -> Option<u32> {
        match self.h.monster_data(unit) {
            Some(m) => m
                .vision
                .map(|r| self.h.vision_seen.get(&r).copied().unwrap_or(0)),
            None => self.h.x.vision_seen(unit),
        }
    }
    /// Marks the vision record seen: its second word := `value` (S == 0),
    /// written only when the record was loaded earlier in the search.
    fn mark_seen(&mut self, unit: UnitId, value: u32) {
        match self.h.monster_data(unit).map(|m| m.vision) {
            Some(Some(r)) => {
                self.h.vision_seen.insert(r, value);
            }
            Some(None) => {}
            None => self.h.x.mark_seen(unit, value),
        }
    }
    /// Clears the forced target on the wired monster data, then the host's part.
    fn ai_reset(&mut self, unit: UnitId) {
        self.clear_target_override(unit);
        self.h.x.ai_reset(unit);
    }
    /// Whether the NPC is interacting, from its interaction list, lent with the quest
    /// parts ([`super::ActionHooks::quest_host`]); else [`Pending`].
    fn interacting(&self, unit: UnitId) -> bool {
        self.h
            .quest_host
            .as_ref()
            .is_some_and(|h| h.npc_interacting(unit))
            || self.h.x.interacting(unit)
    }
    /// Whether the unit is busy: the interact info on the unit record
    /// (active), then [`Pending::busy`] (cursor).
    fn busy(&self, unit: UnitId) -> bool {
        self.units.get(unit).is_some_and(|r| r.interact.active) || self.h.x.busy(unit)
    }
    /// Every monster gets its interaction block at init, so any monster
    /// has one; other units ask [`Pending`].
    fn has_interaction_block(&self, unit: UnitId) -> bool {
        self.units
            .get(unit)
            .is_some_and(|r| r.ty == crate::units::UnitType::Monster)
            || self.h.x.has_interaction_block(unit)
    }
    /// Whether `player` is in the NPC's interaction list, as [`Self::interacting`].
    fn in_interaction_list(&self, npc: UnitId, player: UnitId) -> bool {
        self.h
            .quest_host
            .as_ref()
            .is_some_and(|h| h.npc_in_list(npc, player))
            || self.h.x.in_interaction_list(npc, player)
    }
    /// Life (stat 6) base := value.
    fn set_life(&mut self, unit: UnitId, value: i32) {
        self.set_base(unit, stat::HITPOINTS, value);
    }
    fn stat(&self, unit: UnitId, s: u16) -> i32 {
        View::stat(self, unit, s)
    }
    fn set_stat(&mut self, unit: UnitId, s: u16, value: i32) {
        self.set_base(unit, s, value);
    }
    /// Sets unit flags.
    fn set_unit_flag(&mut self, unit: UnitId, mask: u32) {
        if let Some(r) = self.units.get_mut(unit) {
            r.flags |= mask;
        }
    }
    /// Sets state `s` on or off: s outside 0 … states count − 1 → nothing;
    /// else the toggle, then the update-queue insert whether or not the bit
    /// changed.
    fn set_state(&mut self, game: &mut Game, unit: UnitId, s: u16, on: bool) {
        if usize::from(s) >= self.stats.data().states.count() {
            return;
        }
        View::set_state(self, unit, s, on);
        if let Err(e) = game.lists.queue_update(unit) {
            self.h
                .errors
                .push(WiringError::Unit(crate::units::modes::UnitError::Game(
                    e.into(),
                )));
        }
    }
    fn path_target(&self, unit: UnitId) -> Option<UnitId> {
        if self.h.paths.is_some() {
            return crate::wiring::path::monsters::path_target(self.h, unit);
        }
        self.h.x.path_target(unit)
    }
}

impl<X: Pending> AiModes for View<'_, X> {
    /// A monster mode change toward `target`. False when the mode set
    /// failed.
    ///
    /// TODO: the game falls into the neutral start itself when a start
    /// function fails and reports nothing; "failed" is read as the mode set
    /// returning an error (state 54, bad mode).
    fn change_mode(&mut self, game: &mut Game, unit: UnitId, mode: u8, target: ModeTarget) -> bool {
        self.h.x.set_mode_target(unit, target);
        crate::wiring::path::monsters::stage_request(self.h, unit, target, None);
        let r = self.monster_set_mode(game, unit, u32::from(mode));
        crate::wiring::path::monsters::drop_staged(self.h, unit);
        r
    }
    fn change_mode_path_byte(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        mode: u8,
        target: ModeTarget,
        path_byte: u8,
    ) -> bool {
        self.h.x.set_mode_target(unit, target);
        crate::wiring::path::monsters::stage_request(self.h, unit, target, Some(path_byte));
        let r = self.monster_set_mode(game, unit, u32::from(mode));
        crate::wiring::path::monsters::drop_staged(self.h, unit);
        r
    }
    /// The mode change with the velocity request: every mode but GH consumes it (staged for the
    /// movement set-up of [`crate::wiring::path::monsters`]; dropped without the provider,
    /// which has no path to give it to).
    fn change_mode_with(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        mode: u8,
        target: ModeTarget,
        path_byte: Option<u8>,
        velocity: &mut crate::monsters::ai::VelocityRequest,
    ) -> bool {
        if u32::from(mode) != MODE_GETHIT {
            let v = std::mem::take(velocity);
            crate::wiring::path::monsters::stage_velocity(self.h, unit, v);
        }
        match path_byte {
            Some(b) => self.change_mode_path_byte(game, unit, mode, target, b),
            None => self.change_mode(game, unit, mode, target),
        }
    }
    /// Sets the animation mode directly: no mode start.
    fn set_anim_mode(&mut self, game: &mut Game, unit: UnitId, mode: u8) {
        let r = {
            let mut sim = crate::units::hooks::Sim {
                game,
                units: self.units,
                stats: self.stats,
                data: self.data,
            };
            crate::units::modes::write_mode(&mut sim, &mut *self.h, unit, u32::from(mode))
        };
        if let Err(e) = r {
            self.unit_error(e);
        }
    }
    /// The path step count: the stop distance, with the path provider.
    fn set_path_steps(&mut self, unit: UnitId, steps: i32) {
        if crate::wiring::path::monsters::set_stop_distance(self.h, unit, steps).is_none() {
            self.h.x.set_path_steps(unit, steps);
        }
    }
    fn path_blocked(&self, unit: UnitId) -> bool {
        crate::wiring::path::monsters::path_blocked(self.h, unit)
            .unwrap_or_else(|| self.h.x.path_blocked(unit))
    }
    /// Stops the path, with the path provider.
    fn stop_path(&mut self, unit: UnitId) {
        if crate::wiring::path::monsters::stop_path(self.h, unit).is_none() {
            self.h.x.stop_path(unit);
        }
    }
    fn set_current_skill(&mut self, unit: UnitId, skill: i32) -> bool {
        self.h.x.set_current_skill(unit, skill)
    }
    fn clear_current_skill(&mut self, unit: UnitId) {
        self.h.x.clear_current_skill(unit);
    }
    /// Sets unit flag 0x40.
    fn set_skill_flag(&mut self, unit: UnitId) {
        if let Some(r) = self.units.get_mut(unit) {
            r.flags |= flags::ATTACK_PENDING;
        }
    }
    fn class_has_mode(&self, class: i32, mode: u8) -> bool {
        self.h.x.class_has_mode(class, mode)
    }
    /// Queues the sound ([`crate::units::sound::queue_sound`]: S→C 0x2C in
    /// the tick's client pass), then [`Pending::play_sound`]. Recorded:
    /// `a2-npc-warriv-talk` frame 41, Jerhyn's greeting
    /// `2c 01 01000000 1200`.
    fn play_sound(&mut self, game: &mut Game, unit: UnitId, sound: u32, to: Option<UnitId>) {
        if let Ok(event) = u16::try_from(sound) {
            if let Err(e) = crate::units::sound::queue_sound(game, unit, event, to) {
                self.unit_error(crate::game::GameError::from(e).into());
            }
        }
        self.h.x.play_sound(game, unit, sound, to);
    }
    /// The knockback get-hit branch: the unit's hit class := 0xA0, then the mode change to
    /// get-hit (3). The 0x69 code 6 it sends carries e = 0xA0, a class with no hit overlay, so
    /// the client keeps the knockback's hit overlay running (recorded probe `w5-a-probe-ovl`:
    /// hit class = 0xA0 at the client's code-6 mode set, f276; `draws-fx-bash-cast` t277
    /// `babash` frame 6).
    fn knockback_to_gethit(&mut self, game: &mut Game, unit: UnitId) {
        if let Some(r) = self.units.get_mut(unit) {
            r.hit_class = 0xA0;
        }
        self.monster_set_mode(game, unit, MODE_GETHIT);
    }
    /// Mode 2 (walk) to [`crate::monsters::ai::radius_point`] with path
    /// step count 1, as the walks to coordinates. A point on the unit's own
    /// cell (k = 0 or t on the unit) is still requested: no path, neutral, and the think
    /// at f + `aidel`.
    fn walk_in_radius(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        target: UnitId,
        a: i32,
        b: i32,
        velocity: &mut crate::monsters::ai::VelocityRequest,
    ) -> bool {
        let at = self.h.path_position(unit);
        let to = self.h.path_position(target);
        let size = self.path_size(unit);
        let (x, y) = crate::monsters::ai::radius_point(at, size, to, a, b);
        AiModes::set_path_steps(self, unit, 1);
        self.change_mode_with(game, unit, 2, ModeTarget::Point(x, y), None, velocity)
    }
    /// The operate entry with the monster as operator on the object state ([`super::objects`]);
    /// a quest, waypoint or `todo` route goes to [`Pending::object_route`]. A game without an
    /// object state: [`Pending::operate_door`].
    fn operate_door(&mut self, game: &mut Game, unit: UnitId, door: UnitId) {
        if self.h.objects.is_none() {
            self.h.x.operate_door(game, unit, door);
            return;
        }
        let Some(guid) = game.lists.unit(door).map(|e| e.guid) else {
            return;
        };
        if let Some((_, Some(d))) = self.operate_object(game, Some(unit), guid) {
            if !matches!(d, Dispatch::Done(_)) {
                let room = game.lists.unit(door).and_then(|e| e.room());
                let at = self.h.path_position(door);
                self.object_route(game, ObjectRoute::Operate(d), room, at);
            }
        }
    }
    /// Starts an overlay ([`Pending::overlay`]).
    fn start_overlay(&mut self, unit: UnitId, overlay: i32) {
        self.h.x.overlay(unit, overlay);
    }
    fn set_facing(&mut self, unit: UnitId, dir: i32) {
        self.h.x.set_facing(unit, dir);
    }
}

/// The AI's line-of-sight mask.
const LINE_MASK_AI: u16 = 4;

impl<X: Pending> AiWorld for View<'_, X> {
    fn in_town(&self, game: &Game, room: RoomId) -> bool {
        self.h.drlg.in_town(game, room)
    }
    fn los_draw(&self, game: &Game, room: RoomId) -> bool {
        if self.h.paths.is_none() {
            return self.h.x.los_draw(game, room);
        }
        self.h.drlg.los_draw(game, room)
    }
    /// Whether the grid at the unit's position has a `mask` bit (with the
    /// path provider: the pattern test with the path's pattern and room).
    fn collides(&self, game: &Game, unit: UnitId, mask: u16) -> bool {
        if let Some(d) = self.h.paths.as_ref().and_then(|p| p.dynamic(unit)) {
            return crate::path::collision::pattern_collides(
                &self.h.drlg,
                d.room,
                d.x(),
                d.y(),
                d.pattern,
                mask,
            );
        }
        let (x, y) = self.h.path_position(unit);
        game.lists
            .unit(unit)
            .and_then(|e| e.room())
            .and_then(|r| self.h.drlg.collision(game, r, x, y))
            .is_some_and(|m| m & mask != 0)
    }
    /// The line-of-sight test (mask 4) with the path provider ([`View::units_line_blocked`]);
    /// else [`Pending`].
    fn line_blocked(&self, game: &Game, a: UnitId, b: UnitId) -> bool {
        self.units_line_blocked(game, a, b, LINE_MASK_AI)
            .unwrap_or_else(|| self.h.x.line_blocked(game, a, b))
    }
    fn in_melee_range(&self, game: &Game, a: UnitId, b: UnitId) -> bool {
        match self.monster_in_melee_range(game, a, b, 0) {
            Some(r) => r,
            None => self.h.x.in_melee_range(a, b, 0),
        }
    }
    /// Whether `unit` can reach `target` directly, with the path provider
    /// ([`crate::path::line::can_reach_directly`] on the path positions,
    /// `unit`'s size and room); else [`Pending`].
    fn can_reach_directly(&self, game: &Game, unit: UnitId, target: UnitId) -> bool {
        if self.h.paths.is_none() {
            return self.h.x.can_reach_directly(game, unit, target);
        }
        let (ax, ay) = self.h.path_position(unit);
        let size = self.path_size(unit);
        let b = self.h.path_position(target);
        let d = crate::monsters::ai::distance_full_size((ax, ay), size, b);
        let a = crate::path::line::LineUnit {
            room: game.lists.unit(unit).and_then(|e| e.room()),
            x: ax,
            y: ay,
            size,
        };
        crate::path::line::can_reach_directly(&self.h.drlg, &a, b, d)
    }
    /// The teleport precheck's spot: a spawn point of the unit's own room for its class, and
    /// that room; without a lent monster world, [`Pending::find_spot`].
    fn find_spot(&mut self, game: &mut Game, unit: UnitId) -> Option<(i32, i32, RoomId)> {
        let room = game.lists.unit(unit).and_then(|e| e.room());
        let class = self.units.get(unit).map(|r| r.class as i32);
        if let (Some(room), Some(class)) = (room, class) {
            if let Some(p) = self.spawn_point(game, unit, class) {
                return p.map(|(x, y)| (x, y, room));
            }
        }
        self.h.x.find_spot(game, unit)
    }
    /// The room's ring of the last dead monsters ([`super::monster_death`]):
    /// each slot's GUID as a monster.
    fn last_dead(&self, game: &Game, room: RoomId) -> [Option<UnitId>; 4] {
        game.lists.room(room).map_or([None; 4], |r| {
            r.dead_guids
                .map(|g| game.lists.find_unit(crate::units::UnitType::Monster, g))
        })
    }
    /// The footprint check for a birth at (x, y): by the class's `BaseId`
    /// the point tested and its mask; the room holding it (found from the
    /// unit's room) must exist and have no collision with a size-2 box.
    /// Other classes pass.
    /// Without the path provider the host's answer.
    fn footprint_ok(&self, game: &Game, class: i32, room: Option<RoomId>, x: i32, y: i32) -> bool {
        if self.h.paths.is_none() {
            return self.h.x.footprint_ok(game, class, room, x, y);
        }
        let base = usize::try_from(class)
            .ok()
            .and_then(|c| self.h.tables.combat.monstats.get(c))
            .map(|m| i32::from(m.baseid as i16));
        let (px, py, mask) = match base {
            Some(206) => (x, y + 3, 0x3C01),
            Some(228) => (x, y + 2, 0x3C01),
            Some(298) => (x, y, 0x3C01),
            Some(334) => (x - 2, y - 2, 0x1C0),
            Some(528) => (x + 2, y + 4, 0x3C01),
            _ => return true,
        };
        let Some(r) = room.and_then(|r| self.h.drlg.find_room(game, r, px, py)) else {
            return false;
        };
        crate::path::collision::size_value(&self.h.drlg, Some(r), px, py, 2, mask) == 0
    }
}

impl<X: Pending> AiTargets for View<'_, X> {
    /// The host's player heads, each at the head of the list its node
    /// index names (a head without a player list keeps the place the host
    /// gave it), followed by the nodes units joined to the lists
    /// ([`Game::target_nodes`]).
    fn target_nodes(&self, game: &Game) -> crate::game::TargetLists {
        use crate::game::node;
        let mut nodes = self.h.x.target_nodes(game);
        let host = std::mem::take(&mut nodes.players);
        for (k, list) in host.into_iter().enumerate() {
            let Some(&head) = list.first() else {
                continue;
            };
            let code = self.units.get(head).map_or(node::NONE, |r| r.node_index);
            let code = if node::player_list(code).is_some() {
                code
            } else {
                node::player(k as u32)
            };
            if let Some(dst) = nodes.list_mut(code) {
                if dst.is_empty() {
                    *dst = list;
                }
            }
        }
        let inserted = game.target_nodes.lists();
        for (i, list) in inserted.players.iter().enumerate() {
            if let Some(dst) = nodes.list_mut(crate::game::node::player(i as u32)) {
                dst.extend(list.iter().copied());
            }
        }
        nodes.good.extend(inserted.good.iter().copied());
        nodes.walls.extend(inserted.walls.iter().copied());
        nodes
    }
    /// The forced target on the wired units ([`super::ai_forced`]).
    fn forced_target(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        a: bool,
        s: bool,
        ai: &crate::monsters::ai::AiStore,
    ) -> Option<(UnitId, i32)> {
        self.forced(game, unit, a, s, Some(ai))
    }
    /// The not-evil main search on the wired units ([`super::ai_scan`]):
    /// room scan 5 and the alternative choice.
    fn good_target_search(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        los: bool,
        ai: &crate::monsters::ai::AiStore,
    ) -> Option<(UnitId, i32)> {
        self.good_search(game, unit, los, Some(ai))
    }
    /// The alternative choice on the wired units ([`super::ai_scan`]).
    fn choose_alternative(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        main: Option<(UnitId, i32)>,
        alt: (UnitId, i32),
        ai: &crate::monsters::ai::AiStore,
    ) -> Option<(UnitId, i32)> {
        let s = super::ai_scan::Scan6 {
            main,
            alt: Some(alt),
        };
        self.alternative_choice(game, unit, s, Some(ai))
    }
    /// The secondary search on the wired units ([`super::ai_scan`]).
    fn secondary_target(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        ai: &crate::monsters::ai::AiStore,
    ) -> (Option<UnitId>, i32, bool) {
        self.secondary_search(game, unit, Some(ai))
    }
    /// The nearest player: scan 2 (mode 1: the client players of the
    /// unit's room's near-room list, own room included, in list order)
    /// with d := the full-size distance (the NPC's size subtracted per
    /// axis, clamped at 0); d > 15 → skip. An NPC without the monstats
    /// `interact` flag takes the first such player; with it, the quest
    /// active test ([`AiSummons::npc_wants_interact`], which
    /// sends 0x8A on true) runs for each such player in scan order and
    /// the first true is taken. Taking stops the scan. "Close" when d < 4;
    /// the unit itself when none.
    fn nearest_player(&mut self, game: &mut Game, unit: UnitId) -> (UnitId, bool) {
        use crate::monsters::ai::AiSummons as _;
        use crate::path::collision::CollisionRooms;
        let Some(room) = game.lists.unit(unit).and_then(|e| e.room()) else {
            return (unit, false);
        };
        let d = &self.h.drlg;
        let mut rooms: Vec<RoomId> = (0..d.adjacent_count(room))
            .filter_map(|i| d.adjacent(room, i))
            .collect();
        if !rooms.contains(&room) {
            rooms.insert(0, room);
        }
        let players = super::dying::client_players(game);
        let at = self.h.path_position(unit);
        let size = self.path_size(unit);
        let interact = self.npc_interact(unit).is_some_and(|(_, i)| i);
        let mut scan = Vec::new();
        for r in rooms {
            scan.extend(
                game.lists
                    .room_units(r)
                    .into_iter()
                    .filter(|p| players.contains(p)),
            );
        }
        for p in scan {
            let dist = crate::monsters::ai::distance_full_size(at, size, self.h.path_position(p));
            if dist > 15 {
                continue;
            }
            if !interact || self.npc_wants_interact(game, p, unit) {
                return (p, dist < 4);
            }
        }
        (unit, false)
    }
    fn find_door(&mut self, game: &mut Game, unit: UnitId) -> Option<UnitId> {
        self.h.x.find_door(game, unit)
    }
    /// objects.txt `MonsterOK` from the object state; without one (or
    /// without object data for `door`) [`Pending::door_monster_ok`].
    fn door_monster_ok(&self, door: UnitId) -> bool {
        self.object_monster_ok(door)
            .unwrap_or_else(|| self.h.x.door_monster_ok(door))
    }
    fn special_walk_target(&mut self, game: &mut Game, unit: UnitId) -> Option<(UnitId, i32)> {
        self.h.x.special_walk_target(game, unit)
    }
    fn nearest_evil_monster(&mut self, game: &mut Game, unit: UnitId) -> Option<UnitId> {
        self.h.x.nearest_evil_monster(game, unit)
    }
}

impl<X: Pending> AiQuests for View<'_, X> {
    fn portal_setup(&mut self, game: &mut Game, unit: UnitId, npc: PortalNpc) -> bool {
        match self.cain_portal(game, unit, npc, CainPortalCall::Setup) {
            Some(CainPortalAnswer::Ok(b)) => b,
            _ => self.h.x.portal_setup(game, unit, npc),
        }
    }
    fn spawn_town_portal(&mut self, game: &mut Game, unit: UnitId, npc: PortalNpc) {
        if self
            .cain_portal(game, unit, npc, CainPortalCall::SpawnTown)
            .is_none()
        {
            self.h.x.spawn_town_portal(game, unit, npc);
        }
    }
    fn spawn_outside_portal(&mut self, game: &mut Game, unit: UnitId, npc: PortalNpc) -> bool {
        match self.cain_portal(game, unit, npc, CainPortalCall::SpawnOutside) {
            Some(CainPortalAnswer::Ok(b)) => b,
            _ => self.h.x.spawn_outside_portal(game, unit, npc),
        }
    }
    fn portal_coords(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        npc: PortalNpc,
    ) -> Option<(i32, i32)> {
        match self.cain_portal(game, unit, npc, CainPortalCall::Coords) {
            Some(CainPortalAnswer::Coords(c)) => c,
            _ => self.h.x.portal_coords(game, unit, npc),
        }
    }
    fn drehya_update(&mut self, game: &mut Game) {
        self.h.x.drehya_update(game);
    }
    fn drehya_wait(&mut self, game: &mut Game) -> bool {
        self.h.x.drehya_wait(game)
    }
    /// The lent quest control's A2Q4 hooks; without one (a host without quests) [`Pending`]'s.
    fn jerhyn_palace_active(&mut self, game: &mut Game) -> bool {
        match self.h.quest_host.as_mut() {
            Some(q) => q.jerhyn_palace_active(),
            None => self.h.x.jerhyn_palace_active(game),
        }
    }
    fn jerhyn_npc_state(&mut self, game: &mut Game, unit: UnitId) -> JerhynStep {
        let Some(mut host) = self.h.quest_host.take() else {
            let (a, b) = self.h.x.jerhyn_npc_state(game, unit);
            return JerhynStep::Out(a, b);
        };
        let at = self.h.path_position(unit);
        let r = host.jerhyn_npc_state(game, self, at);
        self.h.quest_host = Some(host);
        r
    }
    fn jerhyn_placed(&mut self, _game: &mut Game) {
        if let Some(q) = self.h.quest_host.as_mut() {
            q.jerhyn_placed();
        }
    }
    fn guard_moving(&mut self, game: &mut Game, unit: UnitId) -> bool {
        match self.h.quest_host.as_mut() {
            Some(q) => q.guard_moving(),
            None => self.h.x.guard_moving(game, unit),
        }
    }
    fn alkor_bird(&mut self, game: &mut Game) -> bool {
        self.h.x.alkor_bird(game)
    }
    fn alkor_reset(&mut self, game: &mut Game) {
        self.h.x.alkor_reset(game);
    }
    fn ormus_altar(&mut self, game: &mut Game) -> Option<(i32, i32)> {
        self.h.x.ormus_altar(game)
    }
    fn ormus_set_altar_mode(&mut self, game: &mut Game) {
        self.h.x.ormus_set_altar_mode(game);
    }
    fn cain_town_coords(&mut self, game: &mut Game, unit: UnitId) -> Option<(i32, i32)> {
        self.h.x.cain_town_coords(game, unit)
    }
    fn cain_in_town_activated(&mut self, game: &mut Game, unit: UnitId) {
        self.h.x.cain_in_town_activated(game, unit);
    }
    /// On the lent quest control inside the think
    /// (`QuestObjectHost::anya_open_portal`), else the Pending.
    fn anya_open_portal(&mut self, game: &mut Game, unit: UnitId) {
        if let Some(mut host) = self.h.quest_host.take() {
            let done = host.anya_open_portal(game, self, unit);
            self.h.quest_host = Some(host);
            if done {
                return;
            }
        }
        self.h.x.anya_open_portal(game, unit);
    }
}

impl<X: Pending> View<'_, X> {
    /// `cain1`'s A1Q4 portal calls on the lent quest control (drehyaiced's
    /// Act V calls are not wired: `None`, the caller takes [`Pending`]'s).
    fn cain_portal(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        npc: PortalNpc,
        call: CainPortalCall,
    ) -> Option<CainPortalAnswer> {
        if npc != PortalNpc::Cain {
            return None;
        }
        let mut host = self.h.quest_host.take()?;
        let r = host.cain_portal(game, self, unit, call);
        self.h.quest_host = Some(host);
        r
    }
}

/// The Act II–V seams: unit flags, max life, state groups and the states
/// count are real; the
/// move mask and the path stop go to the path seams of [`Pending`];
/// everything else to the `ai_*` calls of [`Pending`].
impl<X: Pending> AiActs for View<'_, X> {
    fn unit_flags(&self, unit: UnitId) -> u32 {
        self.units.get(unit).map_or(0, |r| r.flags)
    }
    fn clear_unit_flag(&mut self, unit: UnitId, mask: u32) {
        if let Some(r) = self.units.get_mut(unit) {
            r.flags &= !mask;
        }
    }
    fn max_life(&self, unit: UnitId) -> i32 {
        self.stats.max_life(unit)
    }
    fn max_mana(&self, unit: UnitId) -> i32 {
        self.stats.max_mana(unit)
    }
    fn has_state_group(&self, unit: UnitId, g: u8) -> bool {
        self.stats.has_group(unit, usize::from(g))
    }
    fn states_count(&self) -> i32 {
        self.stats.data().states.count() as i32
    }
    fn has_list_flag(&self, unit: UnitId, flags: u32) -> bool {
        // The unit's list must be extended; then the first list of the
        // active (or parked) chain sharing a flag.
        self.stats
            .unit_list(unit)
            .filter(|&r| self.stats.is_extended(r))
            .is_some_and(|r| self.stats.list_by_flags(r, flags).is_some())
    }
    fn hostile(&self, game: &Game, a: UnitId, b: UnitId) -> bool {
        self.h.x.ai_hostile(game, a, b)
    }
    /// The source-unit link (second flag word bit 0x400: the unit of the
    /// stored type and GUID), else the host's.
    fn owner(&self, game: &Game, unit: UnitId) -> Option<UnitId> {
        if let Some((ty, guid)) = self
            .units
            .get(unit)
            .filter(|r| r.flags2 & 0x400 != 0)
            .map(|r| r.source)
        {
            let ty = *crate::units::UnitType::ALL.get(ty as usize)?;
            return game.lists.find_unit(ty, guid);
        }
        self.h.x.ai_owner(game, unit)
    }
    fn owner_record(&self, unit: UnitId) -> Option<(i32, u32)> {
        self.h.x.ai_owner_record(unit)
    }
    fn quest_flag(&self, player: UnitId, difficulty: u8, quest: i32, flag: i32) -> bool {
        self.h.x.ai_quest_flag(player, difficulty, quest, flag)
    }
    fn portal_guid(&self, player: UnitId) -> Option<u32> {
        self.h.x.ai_portal_guid(player)
    }
    /// Monster data `nComponent[i]` from the lent monster world;
    /// [`Pending::ai_component`] without one.
    fn component(&self, unit: UnitId, i: usize) -> u8 {
        match self.h.monster_data(unit) {
            Some(d) => d.components.get(i).copied().unwrap_or(0),
            None => self.h.x.ai_component(unit, i),
        }
    }
    fn target_unit(&self, game: &Game, unit: UnitId) -> Option<UnitId> {
        self.h.x.ai_target_unit(game, unit)
    }
    /// Sets the forced target on the wired monster data ([`super::ai_forced`]).
    fn set_target_override(&mut self, unit: UnitId, kind: i32, guid: u32) {
        View::set_target_override(self, unit, kind as u32, guid);
    }
    fn chain_index(&self, class: i32) -> i32 {
        self.h.monster_chain_position(class)
    }
    fn class_for_level(&self, game: &Game, room: Option<RoomId>, class: i32) -> i32 {
        self.h.x.ai_class_for_level(game, room, class)
    }
    /// A summoned monster's entry (`ActionHooks::monster_skills`): its
    /// base level (no bonus source on a monster entry); else the seam.
    fn skill_level(&self, unit: UnitId, skill: i32, highest: bool) -> Option<i32> {
        match self.h.monster_skills.get(&unit) {
            Some(m) => m.get(&skill).copied(),
            None => match self.h.natural_skills.get(&unit) {
                Some(m) => m.get(&skill).copied(),
                None => match self.h.skill_lists.get(&unit) {
                    // A player's list (`skill_lists`): the entry's base
                    // level plus bonus; the highest of the unit's entries
                    // of the skill when `highest`, else the native one
                    // (owner −1).
                    Some(l) => l
                        .view()
                        .iter()
                        .filter(|e| e.skill == skill && (highest || e.owner_guid == -1))
                        .map(|e| e.base + e.level_bonus)
                        .max(),
                    None => self.h.x.ai_skill_level(unit, skill, highest),
                },
            },
        }
    }
    /// A summoned monster's entry: the id and its mode, fixed when the
    /// entry is created (a monster's `monanim`).
    fn skill_entry(&self, unit: UnitId, skill: i32) -> Option<(i32, u8)> {
        match self.h.monster_skills.get(&unit) {
            Some(m) => {
                m.get(&skill)?;
                let mode = self.h.tables.skills.skill(skill)?.monanim;
                Some((skill, mode))
            }
            // The monster's init entries (the `Skill1..8` of monstats) are
            // found as well (recorded gen-su-60 draws of Nihlathak's steps
            // 6 and 7).
            None => match self.h.natural_skills.get(&unit) {
                Some(m) if m.contains_key(&skill) => {
                    let mode = self.h.tables.skills.skill(skill)?.monanim;
                    Some((skill, mode))
                }
                _ => self.h.x.ai_skill_entry(unit, skill),
            },
        }
    }
    /// A monster with a skill list (an assigned aura,
    /// [`Pending::monster_right_aura`]): its hand's entry, id and base
    /// level; else the seam.
    fn hand_skill(&self, unit: UnitId, right: bool) -> Option<(i32, i32)> {
        match self.h.skill_lists.get(&unit) {
            Some(l) => {
                let i = if right { l.right } else { l.left };
                i.and_then(|i| l.view().get(i).map(|e| (e.skill, e.base)))
            }
            None => self.h.x.ai_hand_skill(unit, right),
        }
    }
    /// Adds a right-hand skill: an aura goes to
    /// [`Pending::monster_right_aura`] (Duriel), any other skill to the
    /// seam.
    fn add_right_skill(&mut self, game: &mut Game, unit: UnitId, skill: i32, level: i32) {
        if self.h.tables.skills.skill(skill).is_some_and(|r| r.aura) {
            let mut sim = Sim {
                game,
                units: &mut *self.units,
                stats: &mut *self.stats,
                data: self.data,
            };
            X::monster_right_aura(&mut *self.h, &mut sim, unit, skill, level);
        } else {
            self.h.x.ai_add_right_skill(game, unit, skill, level);
        }
    }
    /// Assigns a skill: a monster's entry
    /// (`ActionHooks::assign_monster_skill`); else the seam.
    fn assign_skill(&mut self, game: &mut Game, unit: UnitId, skill: i32, level: i32) {
        if self
            .units
            .get(unit)
            .is_some_and(|r| r.ty == UnitType::Monster)
        {
            let mut sim = Sim {
                game,
                units: &mut *self.units,
                stats: &mut *self.stats,
                data: self.data,
            };
            self.h.assign_monster_skill(&mut sim, unit, skill, level);
            return;
        }
        self.h.x.ai_assign_skill(game, unit, skill, level);
    }
    fn set_skill_param(&mut self, unit: UnitId, skill: i32, value: i32) -> bool {
        self.h.x.ai_set_skill_param(unit, skill, value)
    }
    fn skill_check(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        skill: i32,
        target: Option<UnitId>,
        x: i32,
        y: i32,
    ) -> bool {
        if self.h.paths.is_none() {
            return self.h.x.ai_skill_check(game, unit, skill, target, x, y);
        }
        let world = SkillRooms { v: self, game };
        crate::monsters::ai::skill_check::skill_check(
            &world,
            &self.h.tables.skills.skills,
            unit,
            skill,
            target,
            x,
            y,
        )
    }
    /// The corpse search for unit U around S with `skill` at `level`: no S
    /// or level ≤ 0 → none. The unit find from S's room around S's
    /// position, radius 10, flags 0x1002 (monsters, dead only), with the
    /// corpse test as callback (a monster in mode 12, no `udead` state,
    /// monstats2 `corpseSel`), then the usual filter. The first unit
    /// found. No room for S: the host's answer.
    fn corpse_search(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        target: Option<UnitId>,
        skill: i32,
        level: i32,
    ) -> Option<UnitId> {
        use crate::monsters::init::find::{self, find_flag, FindQuery};
        let s = target?;
        if level <= 0 {
            return None;
        }
        let Some(room) = game.lists.unit(s).and_then(|e| e.room()) else {
            return self.h.x.ai_corpse_search(game, unit, target, skill, level);
        };
        let (x, y) = self.h.path_position(s);
        let q = FindQuery {
            flags: find_flag::MONSTERS | find_flag::DEAD_ONLY,
            x,
            y,
            r: 10,
            ..FindQuery::default()
        };
        let found = find::find_units(&mut AiFinder { v: self, game }, Some(room), &q);
        found.into_iter().find(|&k| self.corpse_any(k))
    }
    fn path_pattern(&self, unit: UnitId) -> i32 {
        self.h.x.ai_path_pattern(unit)
    }
    fn set_path_pattern(&mut self, unit: UnitId, pattern: i32) {
        self.h.x.ai_set_path_pattern(unit, pattern);
    }
    fn set_move_mask(&mut self, unit: UnitId, mask: u16) {
        self.h.x.set_move_mask(unit, mask);
    }
    fn place_unit(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        room: Option<RoomId>,
        x: i32,
        y: i32,
    ) -> bool {
        self.h.x.ai_place_unit(game, unit, room, x, y)
    }
    fn stamp_pattern(
        &mut self,
        game: &mut Game,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        pattern: i32,
        mask: u16,
    ) {
        self.h.x.ai_stamp_pattern(game, room, x, y, pattern, mask);
    }
    fn clear_cell(&mut self, game: &mut Game, room: Option<RoomId>, x: i32, y: i32, bits: u16) {
        self.h.x.ai_clear_cell(game, room, x, y, bits);
    }
    fn point_collides(&self, game: &Game, room: Option<RoomId>, x: i32, y: i32, mask: u16) -> bool {
        self.h.x.ai_point_collides(game, room, x, y, mask)
    }
    fn pattern_collides(&self, game: &Game, unit: UnitId, pattern: i32, mask: u16) -> bool {
        self.h.x.ai_pattern_collides(game, unit, pattern, mask)
    }
    fn free_point(
        &mut self,
        game: &mut Game,
        room: Option<RoomId>,
        x: i32,
        y: i32,
        size: i32,
    ) -> Option<(i32, i32)> {
        self.h.x.ai_free_point(game, room, x, y, size)
    }
    /// A free spot for `class` in the unit's room: x, y are outputs, a
    /// probe-placed point of the room's box on the room seed (the start
    /// point is not read; Baal's teleport, k = 14). Without a
    /// lent monster world, [`Pending::ai_free_spot_for`].
    fn free_spot_for(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        class: i32,
        x: i32,
        y: i32,
    ) -> Option<(i32, i32)> {
        match self.spawn_point(game, unit, class) {
            Some(p) => p,
            None => self.h.x.ai_free_spot_for(game, unit, class, x, y),
        }
    }
    /// The room holding (x, y), searched from the unit's own room.
    fn room_at(&self, game: &Game, unit: UnitId, x: i32, y: i32) -> Option<RoomId> {
        let from = game.lists.unit(unit).and_then(|e| e.room())?;
        self.h
            .drlg
            .find_room(game, from, x, y)
            .or_else(|| self.h.x.ai_room_at(game, unit, x, y))
    }
    fn move_in_radius(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        target: UnitId,
        mode: u8,
        a: i32,
        b: i32,
    ) -> bool {
        self.h.x.ai_move_in_radius(game, unit, target, mode, a, b)
    }
    fn set_path_target(&mut self, unit: UnitId, target: UnitId) {
        self.h.x.ai_set_path_target(unit, target);
    }
    fn path_has_points(&mut self, game: &mut Game, unit: UnitId, target: UnitId) -> bool {
        self.h.x.ai_path_has_points(game, unit, target)
    }
    /// The 64-direction from the unit toward `target` through the path
    /// provider (`View::path_dir64`); else the host's.
    fn direction64(&self, unit: UnitId, target: UnitId) -> i32 {
        let at = self.h.path_position(target);
        self.path_dir64(unit, at)
            .unwrap_or_else(|| self.h.x.ai_direction64(unit, target))
    }
    fn stop_unit_path(&mut self, unit: UnitId) {
        AiModes::stop_path(self, unit);
    }
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
    ) -> Option<UnitId> {
        // Through the lent monster world (`MonsterWorld::spawn_at`): the
        // placement and the creation draw on the game seed as the game's
        // do. Without a lent world, the host's answer.
        let mut sim = Sim {
            game,
            units: self.units,
            stats: self.stats,
            data: self.data,
        };
        let placed = self.h.with_monster_world(|w, h| {
            w.spawn_at(&mut sim, h, room, x, y, class, mode, spread, flags as u16)
        });
        match placed.flatten() {
            Some(unit) => unit,
            None => self
                .h
                .x
                .ai_spawn_monster(sim.game, room, x, y, class, mode, spread, flags),
        }
    }
    /// Lends the AI store to the hooks while the think runs.
    fn lend_ai_store(
        &mut self,
        store: crate::monsters::ai::AiStore,
    ) -> Option<crate::monsters::ai::AiStore> {
        self.h.ai = Some(store);
        None
    }
    fn return_ai_store(&mut self) -> Option<crate::monsters::ai::AiStore> {
        self.h.ai.take()
    }
    /// The kill on this view's parts (the Baal clone with a missing owner:
    /// the death mode on its first think).
    fn kill(&mut self, game: &mut Game, unit: UnitId, killer: Option<UnitId>) {
        let mut cv = self.combat(game);
        super::reaction::kill_by(&mut cv, unit, killer);
    }
    /// The unit leaves its room and is removed (the caged barbarians at
    /// their portal, Baal at the stairs);
    /// every player is told (S→C 0x0A), as the quest host's removal.
    fn remove_unit(&mut self, game: &mut Game, unit: UnitId) {
        let Some((ty, guid)) = game.lists.unit(unit).map(|u| (u.ty as u8, u.guid)) else {
            return self.h.x.ai_remove_unit(game, unit);
        };
        let msg = crate::units::messages::remove_unit(ty, guid);
        for p in game.lists.units_of_type(crate::units::UnitType::Player) {
            self.h.x.send(p, &msg);
        }
        self.remove(game, unit);
    }
    fn link_clone(&mut self, game: &mut Game, unit: UnitId, clone: UnitId) {
        self.h.x.ai_link_clone(game, unit, clone);
    }
    /// Re-initialises the unit as `class` on the lent monster world;
    /// without one, the host's answer.
    fn reinit_class(&mut self, game: &mut Game, unit: UnitId, class: i32, mode: u8) {
        let mut sim = Sim {
            game,
            units: self.units,
            stats: self.stats,
            data: self.data,
        };
        let done = self
            .h
            .with_monster_world(|w, h| w.reinit(&mut sim, h, unit, class, u32::from(mode)));
        if done.is_none() {
            self.h.x.ai_reinit_class(sim.game, unit, class, mode);
        }
    }
    fn change_class_list(&mut self, game: &mut Game, unit: UnitId, class: i32) {
        self.h.x.ai_change_class_list(game, unit, class);
    }
    fn wisp_buff(&mut self, game: &mut Game, target: UnitId, value: i32, expire: i32) {
        self.h.x.ai_wisp_buff(game, target, value, expire);
    }
    /// An 0xA4 record (class u16) on the unit, the unit queued for update.
    fn preload_class(&mut self, game: &mut Game, unit: UnitId, class: i32) {
        let r = super::event_records::EventRecord::Preload {
            class: class as u16,
        };
        self.h.event_records.push(unit, r);
        let _ = game.lists.queue_update(unit);
    }
    /// The unit find for the wisp: room = the unit's room, centre = its
    /// position, r = 32, with a callback. With a callback the usual filter
    /// is not run, so the find yields every unit of the chosen rooms in
    /// found order; the callback test (a live monster with `BaseId` 118) is
    /// the body's. No room: the host's answer.
    fn wisp_find(&mut self, game: &mut Game, unit: UnitId) -> Vec<UnitId> {
        let Some(room) = game.lists.unit(unit).and_then(|e| e.room()) else {
            return self.h.x.ai_wisp_find(game, unit);
        };
        let (x, y) = self.h.path_position(unit);
        let inside = self.h.drlg.subtiles(game, room).is_some_and(|b| {
            crate::monsters::init::find::strictly_inside((b.x, b.y, b.w, b.h), x, y, 32)
        });
        let rooms = if inside {
            vec![room]
        } else {
            game.lists
                .room(room)
                .map_or_else(Vec::new, |r| r.adjacent.clone())
        };
        rooms
            .into_iter()
            .flat_map(|r| game.lists.room_units(r))
            .collect()
    }
    /// Wave `w` (0..=4) is superunique 61 + w (Baal Subject 1..5; the
    /// hcIdx map is the identity): its class from the drop tables'
    /// `superuniques` row, the mapped id = row + the `monstats` count. No
    /// drop tables or row: the host's answer (`Pending::ai_wave`).
    fn wave(&self, w: i32) -> Option<(i32, i32)> {
        let row = 61 + w;
        let found = (0..=4).contains(&w).then_some(()).and_then(|_| {
            let d = self.h.object_drops.as_ref()?;
            let su = d.tables.superuniques.get(row as usize)?;
            let count = self.h.tables.combat.monstats.len() as i32;
            Some((row + count, su.class as i32))
        });
        found.or_else(|| self.h.x.ai_wave(w))
    }
    fn clear_room_portal_flag(&mut self, game: &mut Game, room: Option<RoomId>) {
        self.h.x.ai_clear_room_portal_flag(game, room);
    }
    /// On the lent quest control inside the think (`QuestObjectHost::ai_quest_call`),
    /// else the Pending's answer.
    fn quest_call(&mut self, game: &mut Game, unit: UnitId, call: QuestCall) -> bool {
        if let Some(mut host) = self.h.quest_host.take() {
            let r = host.ai_quest_call(game, self, unit, call);
            self.h.quest_host = Some(host);
            if let Some(r) = r {
                return r;
            }
        }
        self.h.x.ai_quest_call(game, unit, call)
    }
}

/// The summon and minion seams: unit flags 2 and the target-node slot are
/// real; everything else keeps the narrow default of [`AiSummons`] until its owner wires it.
impl<X: Pending> AiSummons for View<'_, X> {
    /// The corpse find: the unit find from the unit's room around its
    /// position, radius n, flags 0x1002 (monsters, dead only), the unit
    /// itself excluded, no callback; the first unit found that passes the
    /// corpse test (a monster in mode 12, monstats2 `corpseSel` and not
    /// `deadCol`, no `udead` state) and is hostile to the unit. No room:
    /// none.
    fn corpse_find(&mut self, game: &mut Game, unit: UnitId, n: i32) -> Option<UnitId> {
        use crate::monsters::init::find::{self, find_flag, FindQuery};
        let room = game.lists.unit(unit).and_then(|e| e.room())?;
        let (x, y) = self.h.path_position(unit);
        let q = FindQuery {
            flags: find_flag::MONSTERS | find_flag::DEAD_ONLY,
            exclude: Some(unit),
            x,
            y,
            r: n,
            ..FindQuery::default()
        };
        let found = find::find_units(&mut AiFinder { v: self, game }, Some(room), &q);
        found
            .into_iter()
            .find(|&k| self.corpse_eatable(k) && AiActs::hostile(self, game, unit, k))
    }
    fn note_spawn_class(&mut self, unit: UnitId, class: i32) {
        self.h.spawn_class_notes.insert(unit, class);
    }
    /// The classes of the unit's level's monster region (the spawn class
    /// pick).
    fn region_classes(&self, game: &Game, unit: UnitId) -> Vec<i32> {
        let level = self.level_id(game, unit);
        let Ok(level) = u32::try_from(level) else {
            return Vec::new();
        };
        self.h
            .monster_world
            .as_ref()
            .and_then(|w| w.region_classes(level))
            .unwrap_or_default()
    }
    /// A skill missile for `owner` (`skill`, `level`, `missile`, x, y):
    /// both coordinates 0 take the owner's target position; still 0, or
    /// farther than 100 (max + min / 2): none; else a missile at the point
    /// (record flags 1) on the real missile store. In the recorded
    /// `ass-blade-sentinel` frame 28 the
    /// creeper's blade missile is the unit allocation that steps the game
    /// seed.
    fn skill_missile(
        &mut self,
        game: &mut Game,
        owner: UnitId,
        skill: i32,
        level: i32,
        missile: i32,
        x: i32,
        y: i32,
    ) -> Option<UnitId> {
        use crate::missiles::{self, MissileBodies, MissileParams};
        let (mut tx, mut ty) = (x, y);
        if tx == 0 && ty == 0 {
            (tx, ty) = MissileBodies::target_position(self, game, owner).unwrap_or((0, 0));
            if tx == 0 && ty == 0 {
                return None;
            }
        }
        let (ox, oy) = self.h.path_position(owner);
        let (dx, dy) = (
            ox.wrapping_sub(tx).wrapping_abs(),
            oy.wrapping_sub(ty).wrapping_abs(),
        );
        if dx.max(dy).wrapping_add(dx.min(dy) / 2) as u32 > 100 {
            return None;
        }
        let p = MissileParams {
            flags: 1,
            owner: Some(owner),
            class: missile,
            x: tx,
            y: ty,
            skill,
            level,
            ..MissileParams::default()
        };
        let mut store = self.h.missiles.take()?;
        let t = self.h.tables.clone();
        let made = {
            let mut cx = missiles::Ctx {
                tables: &t.missiles,
                store: &mut store,
                world: &mut *self,
            };
            missiles::create_missile(game, &mut cx, &p)
        };
        self.h.missiles = Some(store);
        made
    }
    /// `a` stores `b` as its owner (type and id, flag-ex 0x400; the source
    /// map the missile owner lookups read).
    fn link_owner(&mut self, game: &mut Game, a: UnitId, b: UnitId) {
        // A missile's owner is the unit this stores: the recorded
        // `ass-blade-sentinel` gives the blade missile the creeper as owner.
        if let Some(m) = self.h.missiles.as_mut().and_then(|s| s.get_mut(a)) {
            m.owner = self.units.get(b).map(|r| crate::missiles::UnitRef {
                ty: r.ty,
                guid: r.guid,
            });
            return;
        }
        let link = self
            .units
            .get(b)
            .map(|r| r.ty.index() as u32)
            .zip(game.lists.unit(b).map(|e| e.guid));
        if let Some(r) = self.units.get_mut(a) {
            match link {
                Some(l) => {
                    r.source = l;
                    r.flags2 |= 0x400;
                }
                None => {
                    r.source = (0, 0);
                    r.flags2 &= !0x400;
                }
            }
        }
        self.h.unit_source.insert(a, b);
    }
    /// The mode of a monster's entry: the skill's `monanim` (fixed when
    /// the entry is created).
    fn entry_mode(&self, unit: UnitId, skill: i32) -> Option<u8> {
        self.unit_skills(unit)
            .iter()
            .any(|&(s, _)| s == skill)
            .then(|| self.h.tables.skills.skill(skill).map(|r| r.monanim))
            .flatten()
    }
    /// The base level of a monster's entry.
    fn skill_base_level(&self, unit: UnitId, skill: i32) -> Option<i32> {
        if let Some(l) = self.h.skill_lists.get(&unit) {
            return l
                .view()
                .iter()
                .find(|e| e.skill == skill && e.owner_guid == -1)
                .map(|e| e.base);
        }
        self.unit_skills(unit)
            .iter()
            .find(|&&(s, _)| s == skill)
            .map(|&(_, l)| l)
    }
    /// The unit's list of init and summon entries, `(id, level)`.
    fn unit_skills(&self, unit: UnitId) -> Vec<(i32, i32)> {
        match self
            .h
            .monster_skills
            .get(&unit)
            .or_else(|| self.h.natural_skills.get(&unit))
        {
            Some(m) => m.iter().map(|(&s, &l)| (s, l)).collect(),
            None => Vec::new(),
        }
    }
    /// A monster with init or summon entries has a skill list.
    fn has_skill_list(&self, unit: UnitId) -> bool {
        self.h.monster_skills.contains_key(&unit) || self.h.natural_skills.contains_key(&unit)
    }
    fn class_skills(&self, class: i32) -> Vec<i32> {
        crate::skills::list::class_skills(&self.h.tables.skills.skills, class)
    }
    /// The final point of the unit's dynamic path; no dynamic path: (0, 0).
    fn path_final_point(&self, unit: UnitId) -> (i32, i32) {
        let p = self.h.paths.as_ref().and_then(|p| p.dynamic(unit));
        p.map_or((0, 0), |d| {
            let f = d.final_target();
            (f.x, f.y)
        })
    }
    /// The player's position-history ring the placement and the walk step
    /// write; read by the pet move's history walk. No ring: empty.
    fn position_history(&self, player: UnitId) -> (usize, [(i32, i32); 20]) {
        let mut out = [(0, 0); 20];
        let Some(hist) = self.h.paths.as_ref().and_then(|p| p.history.get(&player)) else {
            return (0, out);
        };
        for (o, e) in out.iter_mut().zip(hist.entries.iter()) {
            *o = (e.0 as i32, e.1 as i32);
        }
        (usize::from(hist.next), out)
    }
    /// The target point of the unit's dynamic path; none: (0, 0).
    fn path_target_point(&self, unit: UnitId) -> (i32, i32) {
        let p = self.h.paths.as_ref().and_then(|p| p.dynamic(unit));
        p.map_or((0, 0), |d| {
            let t = d.target();
            (t.x, t.y)
        })
    }
    /// The path target point := (x, y), target unit cleared (DesertTurret).
    fn set_path_target_point(&mut self, unit: UnitId, x: i32, y: i32) {
        self.h.path_set_target_point(unit, x, y);
    }
    /// The 64-direction toward (x, y) through the path provider
    /// (`View::path_dir64`); no provider: 0.
    fn direction64_to(&self, unit: UnitId, x: i32, y: i32) -> i32 {
        self.path_dir64(unit, (x, y)).unwrap_or(0)
    }
    /// Snaps the path direction to `dir` (GargoyleTrap).
    fn snap_direction(&mut self, unit: UnitId, dir: i32) {
        let _ = self.path_snap_direction(unit, dir);
    }
    /// The coordinate-list index, as the population view's: the room holding the point among
    /// `room` and its adjacency array; none (or no room) → 0; that room's record at the point →
    /// its index; no record → −1.
    fn coord_index(&self, game: &Game, room: Option<RoomId>, x: i32, y: i32) -> i32 {
        let Some(at) = room.and_then(|r| self.h.drlg.find_room(game, r, x, y)) else {
            return 0;
        };
        self.h
            .drlg
            .drlg_room(game, at)
            .and_then(|(d, r)| d.coord_at(r, x, y))
            .map_or(-1, |c| c.index as i32)
    }
    /// The palace guard's door hooks from the lent quest control. The Act
    /// V prisoner AI's hooks: the reads from the quest control's published
    /// states ([`Pending::quest_rescue`]); the calls with an effect queued
    /// for it ([`Pending::queue_quest_event`]); the dead-prison-door test is
    /// read here. Other hooks keep the default.
    fn quest_hook(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        player: Option<UnitId>,
        hook: crate::monsters::ai::QuestHook,
    ) -> bool {
        use super::QuestEvent;
        use crate::monsters::ai::QuestHook;
        let guid = game.lists.unit(unit).map_or(0, |e| e.guid);
        match hook {
            // The palace guard's A2Q4 hooks on the lent quest control;
            // none: false. The third is [`AiSummons::palace_guard_point`].
            QuestHook::PalaceDoorOpen => self
                .h
                .quest_host
                .as_mut()
                .is_some_and(|q| q.palace_door_open()),
            QuestHook::PalaceGuardAside => self
                .h
                .quest_host
                .as_mut()
                .is_some_and(|q| q.palace_guard_aside()),
            // The Dark Wanderer's minion hook.
            QuestHook::DarkWandererGone => {
                if let Some(mut host) = self.h.quest_host.take() {
                    host.wanderer_minions(game, self, unit);
                    self.h.quest_host = Some(host);
                }
                false
            }
            QuestHook::WussieLeaving => self.h.x.quest_rescue(guid).0,
            QuestHook::WussieLeave => {
                self.h.x.queue_quest_event(QuestEvent::WussieLeft { guid });
                false
            }
            QuestHook::WussieCanRescue => self.dead_prison_door_near(game, player.unwrap_or(unit)),
            QuestHook::WussieRescue => {
                if let Some(player) = player {
                    self.h
                        .x
                        .queue_quest_event(QuestEvent::WussieRescue { player, unit });
                }
                false
            }
            QuestHook::WussieWait => {
                self.h.x.queue_quest_event(QuestEvent::WussieWait);
                false
            }
            _ => false,
        }
    }
    /// The palace guard's point on the lent quest control: true and the guard's point while one
    /// of its two position flags is set; else 0 and (x, y) untouched (also with no control
    /// lent).
    fn palace_guard_point(&mut self, _game: &mut Game, x: i32, y: i32) -> (bool, i32, i32) {
        match self
            .h
            .quest_host
            .as_mut()
            .and_then(|q| q.palace_guard_point())
        {
            Some((px, py)) => (true, px, py),
            None => (false, x, y),
        }
    }
    /// The Dark Wanderer's walk target on the lent quest
    /// control; none lent: no target.
    fn dark_wanderer_target(&mut self, game: &mut Game, unit: UnitId) -> Option<(i32, i32)> {
        let mut host = self.h.quest_host.take()?;
        let r = host.wanderer_target(game, self, unit);
        self.h.quest_host = Some(host);
        r
    }
    /// The group's portal when spawned and existing.
    fn rescue_portal(&mut self, game: &mut Game, unit: UnitId) -> Option<Option<UnitId>> {
        let guid = game.lists.unit(unit)?.guid;
        let portal = self.h.x.quest_rescue(guid).1?;
        let o = game
            .lists
            .find_unit(crate::units::UnitType::Object, portal)?;
        Some(Some(o))
    }
    /// The quest active test on the game's quest control, lent to the
    /// hooks while the tick runs ([`super::ActionHooks::quest_host`]):
    /// true when an active function wants the player to talk to `unit`
    /// (and `8A 01 <GUID>` was sent). No lent control (a host without
    /// quests): false, no send.
    fn npc_wants_interact(&mut self, game: &mut Game, player: UnitId, unit: UnitId) -> bool {
        let Some((class, interact)) = self.npc_interact(unit) else {
            return false;
        };
        let Some(mut host) = self.h.quest_host.take() else {
            return false;
        };
        let r = host.npc_wants_interact(game, self, player, unit, class, interact);
        self.h.quest_host = Some(host);
        r
    }
    /// Evaluates a skill calc column on the unit (`calc`, `skill`,
    /// `level`).
    fn skill_calc(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        skill: i32,
        calc: u32,
        level: i32,
    ) -> i32 {
        let t = self.h.tables.clone();
        let mut cv = self.combat(game);
        crate::skills::eval_skill(&mut cv, &t.skills, Some(unit), calc, skill, level)
    }
    /// The line-of-sight test with `mask` on the path provider; else the seam's
    /// default (clear).
    fn line_blocked_mask(&self, game: &Game, a: UnitId, b: UnitId, mask: u16) -> bool {
        self.units_line_blocked(game, a, b, mask).unwrap_or(false)
    }
    fn set_unit_flags2(&mut self, unit: UnitId, mask: u32) {
        if let Some(r) = self.units.get_mut(unit) {
            r.flags2 |= mask;
        }
    }
    fn target_slot(&self, unit: UnitId) -> i32 {
        self.units.get(unit).map_or(11, |r| r.node_index as i32)
    }
    /// Registers the unit as the node at the head of list `slot`; the
    /// unit's target-node slot := slot.
    fn register_target_node(&mut self, game: &mut Game, unit: UnitId, slot: i32) {
        let Some(r) = self.units.get_mut(unit) else {
            return;
        };
        let Ok(index) = u8::try_from(slot) else {
            return;
        };
        r.node_index = index.into();
        game.target_nodes.push_front(slot, unit);
    }
    /// From the published hireling facts: the `Id` of the
    /// unit's node when `owner` holds it.
    fn hireling_id(&self, _game: &Game, owner: UnitId, unit: UnitId) -> Option<i32> {
        match self.h.hireling_ai.ids.get(&unit) {
            Some(&(o, id)) if o == owner => i32::try_from(id).ok(),
            _ => None,
        }
    }
    /// The hireling row for (`id`, `level`) on the published rows.
    fn hireling_row(&self, _game: &Game, id: i32, level: i32) -> Option<HireRow> {
        let rows = self.h.hireling_ai.rows.as_ref()?;
        let i = rows.row_at(self.data.expansion, u32::try_from(id).ok()?, level)?;
        rows.rows.get(i).map(|r| r.ai_row())
    }
}

impl<X: Pending> View<'_, X> {
    /// The melee reach of a monster: its monstats2 `MeleeRng`, or for 255
    /// the weapon class in the current mode (through the disguise remap):
    /// 2 for `2ht` (class 6), else 0. A
    /// monster's class is hth in DT (0) and DD (12) unless its monstats2
    /// `compositeDeath` is set, else its monstats2 `BaseW`; no monstats2
    /// row is hth. A monster drawn as a player reads the host's composit
    /// class. `None`: no monstats / monstats2 row for the unit's class.
    pub(crate) fn monster_reach(&self, a: UnitId) -> Option<i32> {
        let t = &self.h.tables.combat;
        let row2 = |class: u32| {
            let ex = t.monstats.get(usize::try_from(class).ok()?)?.monstatsex;
            t.monstats2.get(usize::from(ex))
        };
        let rng = row2(self.units.get(a)?.class)?.meleerng;
        if rng != 255 {
            return Some(i32::from(rng));
        }
        let two_ht = match self.h.draw_identity_in(self.units, self.stats, a)? {
            (UnitType::Monster, class, mode) => row2(class).is_some_and(|r| {
                (!matches!(mode, 0 | 12) || r.compositedeath) && r.basew == *b"2ht "
            }),
            _ => self.h.x.composit_weapon_class(a) == crate::combat::range::WEAPON_CLASS_2HT,
        };
        Some(if two_ht { 2 } else { 0 })
    }

    /// Whether monster `a` is in melee range of `b` (with the `extra`
    /// reach), with the path provider and a monstats2 row: reach
    /// `MeleeRng` + 1 against the unit distance, then
    /// the collision line (mask 0x804). `None`: not answerable here (the
    /// host answers).
    pub(crate) fn monster_in_melee_range(
        &self,
        game: &Game,
        a: UnitId,
        b: UnitId,
        extra: i32,
    ) -> Option<bool> {
        let paths = self.h.paths.as_ref()?;
        let t = &self.h.tables.combat;
        let ar = self.units.get(a)?;
        let reach = if ar.ty == UnitType::Player {
            // For a player: the weapon's `rangeadder`, the host's
            // `melee_range` (0 without one).
            self.h.x.melee_range(a)
        } else {
            Some(ar).filter(|r| r.ty == UnitType::Monster)?;
            self.monster_reach(a)?
        };
        let dist = |a: UnitId, b: UnitId| {
            let pt = |u: UnitId| {
                let (x, y) = self.h.path_position(u);
                crate::path::Point { x, y }
            };
            crate::path::walk::geom::unit_distance(
                &paths.tables,
                pt(a),
                self.path_size(a),
                pt(b),
                self.path_size(b),
            )
        };
        // Step 2: the tentacle classes.
        let tentacle = self
            .units
            .get(b)
            .filter(|r| r.ty == UnitType::Monster)
            .and_then(|r| {
                t.monstats
                    .get(usize::try_from(r.class).ok()?)
                    .map(|m| m.baseid)
            });
        if matches!(tentacle, Some(258 | 261)) && reach + 8 > dist(a, b) {
            return Some(true);
        }
        // Step 3.
        let d = dist(a, b);
        if d <= 0 {
            return Some(true);
        }
        if reach.wrapping_add(extra).wrapping_add(1) < d {
            return Some(false);
        }
        Some(!self.units_line_blocked(game, a, b, 0x804).unwrap_or(false))
    }
}

/// The `SkillCheckWorld` of the AI on the DRLG rooms and the path
/// records; used once the host has turned the path provider on.
struct SkillRooms<'a, 'v, X: Pending> {
    v: &'a View<'v, X>,
    game: &'a Game,
}

impl<X: Pending> crate::monsters::ai::skill_check::SkillCheckWorld for SkillRooms<'_, '_, X> {
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.v.h.path_position(unit)
    }
    fn room(&self, unit: UnitId) -> Option<RoomId> {
        self.game.lists.unit(unit).and_then(|e| e.room())
    }
    fn has_unit_flag(&self, unit: UnitId, bit: u32) -> bool {
        self.v.units.get(unit).is_some_and(|r| r.flags & bit != 0)
    }
    fn dead_or_dying(&self, unit: UnitId) -> bool {
        self.v
            .units
            .get(unit)
            .is_some_and(|r| r.mode == 0 || r.mode == 12)
    }
    fn in_town(&self, room: RoomId) -> bool {
        self.v.h.drlg.in_town(self.game, room)
    }
    fn pattern_free(&self, room: Option<RoomId>, unit: UnitId, x: i32, y: i32, mask: u16) -> bool {
        let pattern = self
            .v
            .h
            .paths
            .as_ref()
            .and_then(|p| p.dynamic(unit))
            .map_or(0, |d| d.pattern);
        !crate::path::collision::pattern_collides(&self.v.h.drlg, room, x, y, pattern, mask)
    }
    fn free_point_room(
        &self,
        room: Option<RoomId>,
        unit: UnitId,
        p: (i32, i32),
        mask: u16,
    ) -> Option<RoomId> {
        let mut pt = crate::path::Point::new(p.0, p.1);
        crate::path::search::free_point(
            &crate::wiring::path::place::Rooms(&self.v.h.drlg),
            room,
            &mut pt,
            self.v.path_size(unit),
            u32::from(mask),
            false,
        )
        .ok()
        .flatten()
    }
    fn line_clear(&self, from: (i32, i32), to: (i32, i32), room: RoomId, mask: u16) -> bool {
        !crate::path::line::line_test(
            &self.v.h.drlg,
            Some(room),
            crate::path::Point::new(from.0, from.1),
            crate::path::Point::new(to.0, to.1),
            mask,
        )
        .blocked()
    }
    fn room_at(&self, unit: UnitId, x: i32, y: i32) -> Option<RoomId> {
        let from = self.room(unit)?;
        self.v.h.drlg.find_room(self.game, from, x, y)
    }
    fn diab_prison_ok(&self, _: Option<RoomId>, _: Option<UnitId>) -> bool {
        // Skill 199 (DiabPrison): the placement test is not wired.
        false
    }
}

impl<X: Pending> View<'_, X> {
    /// The corpse test for eating: a monster in mode 12, monstats2
    /// `deadCol` off and `corpseSel` on, in no `udead` state.
    fn corpse_eatable(&self, t: UnitId) -> bool {
        let Some(r) = self.units.get(t) else {
            return false;
        };
        r.ty == UnitType::Monster
            && r.mode == 12
            && self
                .h
                .tables
                .combat
                .monstats2(r.class as i32)
                .is_some_and(|m| !m.deadcol && m.corpsesel)
            && !self.h.x.udead_state(t)
    }
    /// The general corpse test: a monster in mode 12, in no `udead` state, its monstats2
    /// `corpseSel`.
    fn corpse_any(&self, t: UnitId) -> bool {
        let Some(r) = self.units.get(t) else {
            return false;
        };
        r.ty == UnitType::Monster
            && r.mode == 12
            && !self.h.x.udead_state(t)
            && self
                .h
                .tables
                .combat
                .monstats2(r.class as i32)
                .is_some_and(|m| m.corpsesel)
    }
}

/// [`crate::monsters::init::find::FindWorld`] over the AI view (the
/// corpse search of [`AiActs::corpse_search`]).
struct AiFinder<'a, 'v, X: Pending> {
    v: &'a View<'v, X>,
    game: &'a Game,
}

impl<X: Pending> crate::monsters::init::find::FindWorld for AiFinder<'_, '_, X> {
    fn room_box(&self, room: RoomId) -> Option<(i32, i32, i32, i32)> {
        let t = self.v.h.drlg.subtiles(self.game, room)?;
        Some((t.x, t.y, t.w, t.h))
    }
    fn adjacent(&self, room: RoomId) -> Vec<RoomId> {
        self.game
            .lists
            .room(room)
            .map_or_else(Vec::new, |r| r.adjacent.clone())
    }
    fn room_in_town(&self, room: RoomId) -> bool {
        self.v.h.drlg.in_town(self.game, room)
    }
    fn room_units(&self, room: RoomId) -> Vec<UnitId> {
        self.game.lists.room_units(room)
    }
    fn unit_type(&self, unit: UnitId) -> Option<UnitType> {
        self.v.units.get(unit).map(|r| r.ty)
    }
    fn mode(&self, unit: UnitId) -> u32 {
        self.v.units.get(unit).map_or(0, |r| r.mode)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.v.h.path_position(unit)
    }
    fn unit_flags(&self, unit: UnitId) -> u32 {
        self.v.units.get(unit).map_or(0, |r| r.flags)
    }
    fn is_undead(&self, unit: UnitId) -> bool {
        self.v.h.is_undead(unit)
    }
    /// No caller here passes flag 0x08 (missiles).
    fn missile_explosion(&self, _: UnitId) -> Option<bool> {
        None
    }
    /// No caller here passes flag 0x200 (the line test).
    fn line_blocked(&self, _: (i32, i32), _: UnitId) -> bool {
        true
    }
}
