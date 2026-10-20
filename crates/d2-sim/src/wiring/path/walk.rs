//! [`PathCtx`]: the one context `path::walk` runs on. It holds the game
//! (lists, timers, frame), the unit records, stat lists and the action
//! state (DRLG, path records, tables, [`Pending`]) and implements
//! [`PathWorld`] (path records, footprints, room lists, clients),
//! [`CollisionRooms`] (the DRLG, [`super::rooms`]), [`WalkUnits`] (unit
//! fields, modes and timers through the unit system, stats, charstats /
//! monstats velocities) and [`PathMotion`] (set position and reset for
//! the teleport that moves a dynamic path to a new cell).
//!
//! Entry points: [`PathCtx::walk_to`] (the point-form mode request),
//! [`walk_message`] (C→S 0x01–0x04, the click-to-walk/run messages), [`player_step`]
//! (event 0 of the walk/run modes 2, 3, 6, 19), [`unit_step`]
//! (one step of any unit's path), [`build_path`] and
//! [`PathCtx::teleport`].

use std::sync::Arc;

use crate::drlg::{CollisionGrid, TileRect};
use crate::game::Game;
use crate::path::coords::{to_fp16_center, Point};
use crate::path::footprint::{teleport, PathMotion};
use crate::path::history::PositionHistory;
use crate::path::walk::request::{handle_message, request, Outcome, WalkTarget};
use crate::path::walk::seams::{PathWorld, StartTarget, UsedSkill, WalkUnits};
use crate::path::walk::{Step, Walk, WalkError};
use crate::path::{CollisionRooms, DynamicPath, PathTables, UnitPath};
use crate::rng::Seed;
use crate::tick::TickRate;
use crate::units::{ClientId, RoomId, UnitId, UnitType};
use crate::wiring::action::reaction::UNIT_FLAG_SOFT_HIT;
use crate::wiring::action::{Pending, View, WiringError};

/// The walk context ([module doc](self)).
pub struct PathCtx<'a, X> {
    pub v: View<'a, X>,
    pub game: &'a mut Game,
}

impl<'a, X: Pending> PathCtx<'a, X> {
    /// A context over a view's parts and `game`.
    pub fn of(v: &'a mut View<'_, X>, game: &'a mut Game) -> Self {
        Self {
            v: View::of(&mut *v.units, &mut *v.stats, v.data, &mut *v.h),
            game,
        }
    }

    /// The path tables (the provider is on whenever a context is used).
    fn tables(&self) -> Arc<PathTables> {
        self.v
            .h
            .paths
            .as_ref()
            .map(|p| p.tables.clone())
            .expect("path provider on")
    }

    /// The speed +0x4C of a run start after its run list is attached
    /// (the run start sets the speed before the list exists): the movement rate
    /// reads stat 67 with the run list, so a
    /// level-1 Amazon's run speed is 101 · 150 / 100 = 151. The mode set
    /// ([`crate::units::mode_set`]) computed it before the attach; this
    /// recomputes it with the list in place.
    pub(crate) fn refresh_run_rate(&mut self, unit: UnitId) {
        let rate = {
            let sim = crate::units::hooks::Sim {
                game: &mut *self.game,
                units: &mut *self.v.units,
                stats: &mut *self.v.stats,
                data: self.v.data,
            };
            self.v.h.movement_rate(&sim, unit)
        };
        if let (Some(s), Some(r)) = (rate, self.v.units.get_mut(unit)) {
            r.anim.speed = s;
        }
    }

    fn walk_error(&mut self, e: WalkError) {
        self.v.h.errors.push(WiringError::Walk(e));
    }

    /// Player mode request, point form `(unit, no skill, mode, x, y,
    /// re-entry 0)`.
    pub fn walk_to(&mut self, unit: UnitId, mode: u32, x: i32, y: i32) -> Option<Outcome> {
        let t = self.tables();
        let target = WalkTarget::Point(Point::new(x, y));
        match request(&t, self, unit, None, mode, target, false) {
            Ok(o) => Some(o),
            Err(e) => {
                self.walk_error(e);
                None
            }
        }
    }

    /// Player mode request, unit form `(unit, skill, mode 3,
    /// type, GUID, 0)`: the run to a unit a skill
    /// use asks for out of reach; the skill
    /// is remembered as the unit's used skill.
    pub fn run_to_unit(&mut self, unit: UnitId, target: UnitId, skill: u16) -> Option<Outcome> {
        let t = self.tables();
        let (ty, guid) = {
            let r = self.v.units.get(target)?;
            (r.ty, r.guid)
        };
        let target = WalkTarget::Unit { ty, guid };
        match request(&t, self, unit, Some(skill), 3, target, false) {
            Ok(o) => Some(o),
            Err(e) => {
                self.walk_error(e);
                None
            }
        }
    }

    /// The NPC approach: the run
    /// request to a unit with no skill, unit form `(no skill, mode 3,
    /// type, GUID, 0)`; it clears the queued action (`clear_queued_action`).
    pub fn approach_unit(&mut self, unit: UnitId, target: UnitId) -> Option<Outcome> {
        let t = self.tables();
        let (ty, guid) = {
            let r = self.v.units.get(target)?;
            (r.ty, r.guid)
        };
        match request(
            &t,
            self,
            unit,
            None,
            3,
            WalkTarget::Unit { ty, guid },
            false,
        ) {
            Ok(o) => Some(o),
            Err(e) => {
                self.walk_error(e);
                None
            }
        }
    }

    /// The player's neutral start: the
    /// ENDANIM handler that runs after a step result 2, even when the
    /// queued interaction of the player step started a new run in between
    /// (a queued object operate fires only on that stop).
    pub fn neutral_start(&mut self, unit: UnitId) {
        if let Some(path) = self.load_path(unit) {
            crate::path::walk::request::neutral_start(self, unit, &path);
        }
    }

    /// Teleport of a unit with a dynamic path to `(room, x, y)` (the path-placement
    /// teleport); a static path is set
    /// directly with its footprint moved.
    ///
    /// TODO: how a unit
    /// with a static path (objects, items) teleports is not settled; the static
    /// set with the footprint removed and added again is used.
    pub fn teleport(&mut self, unit: UnitId, room: Option<RoomId>, x: i32, y: i32) {
        let missile = self.unit_type(unit) == UnitType::Missile;
        let rec = self
            .v
            .h
            .paths
            .as_ref()
            .and_then(|p| p.record(unit))
            .cloned();
        match rec {
            Some(UnitPath::Dynamic(mut d)) => {
                let r = teleport(self, &mut d, missile, room, x, y);
                self.store_path(unit, &d);
                if let Err(e) = r {
                    self.v.h.errors.push(WiringError::Path(e));
                }
            }
            Some(UnitPath::Static(mut s)) => {
                self.v.path_remove_footprint(unit, true);
                s.set(room, x, y);
                if let Some(p) = self.v.h.paths.as_mut() {
                    p.records.insert(unit, UnitPath::Static(s));
                }
                self.v.path_add_footprint(unit);
            }
            None => {}
        }
    }

    /// Teleport and clear:
    /// the teleport ([`PathCtx::teleport`]), then the path's point count
    /// := 0.
    pub fn teleport_clear(&mut self, unit: UnitId, room: Option<RoomId>, x: i32, y: i32) {
        self.teleport(unit, room, x, y);
        if let Some(d) = self.v.h.paths.as_mut().and_then(|p| p.dynamic_mut(unit)) {
            d.point_count = 0;
        }
    }

    /// Room-change messages for the unit (removals and adds for the clients whose
    /// view of the unit changed).
    pub fn room_change_messages(&mut self, unit: UnitId) {
        let Some(mut d) = self.load_path(unit) else {
            return;
        };
        let t = self.tables();
        Walk { t: &t, c: self }.room_change_messages(unit, &mut d);
        self.store_path(unit, &d);
    }

    /// Unit step: one movement step along the unit's path.
    pub fn step(&mut self, unit: UnitId) -> Option<Step> {
        let mut d = self.load_path(unit)?;
        let t = self.tables();
        let r = Walk { t: &t, c: self }.step(unit, &mut d);
        self.store_path(unit, &d);
        match r {
            Ok(s) => Some(s),
            Err(e) => {
                self.walk_error(e);
                None
            }
        }
    }
}

/// C→S 0x01–0x04 after the intent gate accepts it
/// (walk/run to a point or to a unit): (x, y) or (type, GUID) in `a`, `b`. The handler
/// result is 0 in every case; the outcome for callers and tests.
pub fn walk_message<X: Pending>(
    v: &mut View<'_, X>,
    game: &mut Game,
    player: UnitId,
    id: u8,
    a: u32,
    b: u32,
) -> (u32, Option<Outcome>) {
    let mut c = PathCtx::of(v, game);
    let t = c.tables();
    match handle_message(&t, &mut c, player, id, a, b) {
        Ok(r) => r,
        Err(e) => {
            c.walk_error(e);
            (0, None)
        }
    }
}

/// C→S 0x5F UpdatePlayerPos: the
/// handler result and what it did (`None`: a fatal path, logged in
/// `ActionHooks::errors`, result 0).
pub fn resync_message<X: Pending>(
    v: &mut View<'_, X>,
    game: &mut Game,
    player: UnitId,
    msg: &[u8],
) -> (u32, Option<crate::path::walk::resync::Resync>) {
    let mut c = PathCtx::of(v, game);
    let t = c.tables();
    match crate::path::walk::resync::handle_resync(&t, &mut c, player, msg) {
        Ok((r, what)) => (r, Some(what)),
        Err(e) => {
            c.walk_error(e);
            (0, None)
        }
    }
}

/// Player mode request (point form, re-entry 1): the
/// player mode request with the interrupt gate skipped (C→S 0x41 starts
/// mode 1 this way).
/// `None`: a fatal path (logged).
pub fn request_skip_gate<X: Pending>(
    v: &mut View<'_, X>,
    game: &mut Game,
    player: UnitId,
    mode: u32,
    x: i32,
    y: i32,
) -> Option<Outcome> {
    let mut c = PathCtx::of(v, game);
    let t = c.tables();
    let target = WalkTarget::Point(Point::new(x, y));
    match request(&t, &mut c, player, None, mode, target, true) {
        Ok(o) => Some(o),
        Err(e) => {
            c.walk_error(e);
            None
        }
    }
}

/// Player mode request, point form / unit
/// form) with re-entry 0 for a caller outside the
/// walk code (the damage reaction's knockback run). `None`: a
/// fatal path (logged).
pub fn player_request<X: Pending>(
    v: &mut View<'_, X>,
    game: &mut Game,
    player: UnitId,
    skill: Option<u16>,
    mode: u32,
    target: WalkTarget,
) -> Option<Outcome> {
    let mut c = PathCtx::of(v, game);
    let t = c.tables();
    match request(&t, &mut c, player, skill, mode, target, false) {
        Ok(o) => Some(o),
        Err(e) => {
            c.walk_error(e);
            None
        }
    }
}

/// Player mode request, unit form with re-entry 1
/// (the interrupt gate skipped): the Swing of the skill bodies,
/// which walks to its target first. `None`: a fatal path (logged).
pub fn player_request_reentry<X: Pending>(
    v: &mut View<'_, X>,
    game: &mut Game,
    player: UnitId,
    skill: Option<u16>,
    mode: u32,
    target: WalkTarget,
) -> Option<Outcome> {
    let mut c = PathCtx::of(v, game);
    let t = c.tables();
    match request(&t, &mut c, player, skill, mode, target, true) {
        Ok(o) => Some(o),
        Err(e) => {
            c.walk_error(e);
            None
        }
    }
}

/// Player event 0 of modes 2, 3, 6, 19 (walk, run, town walk, knockback):
/// the player's step; the action result for the mode system (2 = stopped).
pub fn player_step<X: Pending>(v: &mut View<'_, X>, game: &mut Game, unit: UnitId) -> u32 {
    let mut c = PathCtx::of(v, game);
    let t = c.tables();
    match (Walk { t: &t, c: &mut c }).player_event0(unit) {
        Ok(s) => {
            // A queued object operate fires only on this stop (step result 2 of
            // the player step), never while the player is still moving.
            if s == Step::Stopped && !c.v.h.player_stops.contains(&unit) {
                c.v.h.player_stops.push(unit);
            }
            s as u32
        }
        Err(e) => {
            c.walk_error(e);
            1
        }
    }
}

/// Unit step: false when it returns 2 (no movement).
pub fn unit_step<X: Pending>(v: &mut View<'_, X>, game: &mut Game, unit: UnitId) -> bool {
    PathCtx::of(v, game).step(unit) == Some(Step::Moving)
}

/// Path compute `(path, unit, town access)`
/// toward the path's target.
// The missile build passes town access 0; the missile branch of the
// path compute returns before it is read.
pub fn build_path<X: Pending>(v: &mut View<'_, X>, game: &mut Game, unit: UnitId) {
    let mut c = PathCtx::of(v, game);
    let Some(mut d) = c.load_path(unit) else {
        return;
    };
    let t = c.tables();
    let r = crate::path::walk::find::compute(&t, &mut c, &mut d, unit, false);
    c.store_path(unit, &d);
    if let Err(e) = r {
        c.walk_error(e);
    }
}

/// The player part of the per-unit update message, for one
/// client of the per-client update (the 0x15 resync first, then the mode
/// messages): S→C 0x15 when the unit's flags 2 ask
/// for it, then, for a player whose mode changed (unit
/// flag 0x1), the mode's row of the player mode-update table
/// ([`mode_update_messages`]: the walk modes' 0x0F / 0x10, the 0x0D rows,
/// the skill modes' skill message 0x4C / 0x4D,
/// [`crate::wiring::action::unit_update::skill_message::player_skill_mode`];
/// every row is described on [`mode_update_messages`]). Sent through
/// [`Pending::send`] to the client's player. Units other than players
/// with a dynamic path, and clients without a player, get nothing.
///
/// The room clean-up at the end of the tick
/// ([`crate::wiring::action::View::room_cleanup`])
/// clears unit flag 0x1 and flags 2 bits 0x10000 / 0x800 after the client
/// pass, so each message goes out once per change.
pub fn update_messages<X: Pending>(
    v: &mut View<'_, X>,
    game: &Game,
    client: ClientId,
    unit: UnitId,
) {
    use crate::path::walk::messages::{reassign_flag, reassign_player};
    let Some(receiver) = game.lists.client(client).and_then(|c| c.player) else {
        return;
    };
    let Some(r) = v.units.get(unit) else {
        return;
    };
    if r.ty != UnitType::Player {
        return;
    }
    let (ty, guid, flags, flags2) = (r.ty as u8, r.guid, r.flags, r.flags2);
    let Some(path) = v.h.paths.as_ref().and_then(|p| p.dynamic(unit)).cloned() else {
        // A unit without a path record (a corpse): only its mode's update
        // function, from the unit's position.
        if flags & crate::units::record::flags::CHANGED != 0 {
            mode_update_messages(v, game, receiver, unit);
        }
        return;
    };
    let own = receiver == unit;
    if let Some(flag) = reassign_flag(flags2, own) {
        let msg = reassign_player(ty, guid, path.x() as u16, path.y() as u16, flag);
        v.h.x.send(receiver, &msg);
    }
    // Message order for one unit: the 0x15 resync, then the item messages,
    // then the mode messages; the host
    // sends them at this mark.
    if v.h.item_marks {
        let mut m = [crate::wiring::action::PLAYER_ITEMS_MARK; 5];
        m[1..].copy_from_slice(&guid.to_le_bytes());
        v.h.x.send(receiver, &m);
    }
    if flags & crate::units::record::flags::CHANGED != 0 {
        mode_update_messages(v, game, receiver, unit);
    }
}

/// Unit flag 0x400 → the sound message, after the event records and before the
/// overhead text and the soft hit (so the level-up Nova sound 0x9A goes out
/// before the level-up overlay 0x2C in the same frame); sent by the host at
/// this mark. A player with a
/// dynamic path only.
pub fn sound_mark<X: Pending>(v: &mut View<'_, X>, game: &Game, client: ClientId, unit: UnitId) {
    let Some(receiver) = game.lists.client(client).and_then(|c| c.player) else {
        return;
    };
    let Some(r) = v.units.get(unit) else {
        return;
    };
    if r.ty != UnitType::Player || !v.h.item_marks {
        return;
    }
    let guid = r.guid;
    if v.h.paths.as_ref().and_then(|p| p.dynamic(unit)).is_none() {
        return;
    }
    let mut m = [crate::wiring::action::PLAYER_SOUND_MARK; 5];
    m[1..].copy_from_slice(&guid.to_le_bytes());
    v.h.x.send(receiver, &m);
}

/// The mode's update function of the player mode-update table for `unit`, sent
/// to `receiver`: run by the
/// per-unit update when the mode changed and, unconditionally, by the add
/// messages of a player (when a client first sees the player).
pub fn mode_update_messages<X: Pending>(
    v: &mut View<'_, X>,
    game: &Game,
    receiver: UnitId,
    unit: UnitId,
) {
    use crate::path::walk::messages::mode_update;
    use crate::wiring::action::unit_update::skill_message::player_skill_mode;
    let Some(r) = v.units.get(unit) else {
        return;
    };
    if r.ty != UnitType::Player {
        return;
    }
    let (ty, guid, mode) = (r.ty as u8, r.guid, r.mode);
    let Some(path) = v.h.paths.as_ref().and_then(|p| p.dynamic(unit)).cloned() else {
        // No path record (a corpse, mode DD): the stop row at the unit's
        // position.
        if let Some(StopRow::Stop(code)) = stop_row(mode) {
            let (x, y) = v.h.path_position(unit);
            let (b, life) = (v.unit_b0(unit), life_percent(v, unit));
            let m = crate::path::walk::messages::player_stop(
                ty, guid, code, x as u16, y as u16, b, life,
            );
            v.h.x.send(receiver, &m);
        }
        return;
    };
    let own = receiver == unit;
    {
        let walk_row = crate::path::walk::messages::mode_codes(mode).is_some();
        if walk_row {
            // The 0x0D rows read T only for the other clients, never the own client.
            if !own {
                let target = path_target(v, game, receiver, unit);
                if let Some(msg) = mode_update(mode, ty, guid, &path, target, own) {
                    v.h.x.send(receiver, &msg);
                }
            }
        } else if let Some(row) = stop_row(mode) {
            stop_row_messages(v, receiver, unit, (ty, guid), &path, row, own);
        } else if player_skill_mode(mode) && (!own || own_skill_message(v, unit)) {
            // The skill rows send the skill message with the
            // path's target unit, else its target point. The own client
            // gets it only with E flags bit 0x4
            // (the client starts its own cast at the click, so it must not get it
            // twice).
            let target = path_target(v, game, receiver, unit);
            v.skill_message(
                game,
                receiver,
                unit,
                (ty, guid),
                target,
                (path.target_x, path.target_y),
            );
        }
    }
}

/// The skill message's target T for the client whose player is
/// `receiver`: the path target unit (a stale or picked-up target is cleared
/// first; the unit itself is
/// none), kept only when its room's client array holds that client
/// (the receiver can see it). The monster update reads it the same way
/// ([`View::mode_target`]). A receiver without a client: none.
fn path_target<X: Pending>(
    v: &mut View<'_, X>,
    game: &Game,
    receiver: UnitId,
    unit: UnitId,
) -> Option<(u8, u32)> {
    let client = game
        .lists
        .clients()
        .into_iter()
        .find(|&c| game.lists.client(c).and_then(|e| e.player) == Some(receiver))?;
    v.mode_target(game, client, unit)
}

/// The skill message for the unit's own client:
/// sent only when the used skill's runtime E flags
/// have bit 0x4 (the dodge / avoid reaction). No used skill → false.
fn own_skill_message<X: Pending>(v: &View<'_, X>, unit: UnitId) -> bool {
    v.h.used_skill_of(unit)
        .is_some_and(|e| v.h.x.entry_flags(unit, &e) & 0x4 != 0)
}

/// The non-walk, non-skill rows of the player mode-update table
/// (stand, death, knockback and the other plain mode changes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StopRow {
    /// NU 1 / TN 5 (code 7): 0x0D with b = 0, other
    /// clients only.
    Neutral,
    /// GH 4 (6), BL 9 (0x12), DD 17 (9): 0x0D, b = the unit's
    /// hit class ([`View::unit_b0`]), every client.
    Stop(u8),
    /// DT 0 (code 8): as `Stop`, then the own client gets
    /// the stat 175 message.
    Death,
    /// KB 19 (code 0x14): 0x0F with byte @11 = the unit's
    /// hit class, every client.
    Knockback,
}

fn stop_row(mode: u32) -> Option<StopRow> {
    Some(match mode {
        1 | 5 => StopRow::Neutral,
        4 => StopRow::Stop(6),
        9 => StopRow::Stop(0x12),
        17 => StopRow::Stop(9),
        0 => StopRow::Death,
        19 => StopRow::Knockback,
        _ => return None,
    })
}

/// The life percent: trunc(100 * (life >> 8) / (max >> 8)),
/// 0 when the maximum is below 256.
fn life_percent<X: Pending>(v: &View<'_, X>, unit: UnitId) -> u8 {
    let m = v.stats.max_life(unit) >> 8;
    if m <= 0 {
        return 0;
    }
    ((v.stats.unit_total(unit, 6, 0) >> 8).wrapping_mul(100) / m) as u8
}

/// Unit flag 0x8000 (the soft hit: damage too small for a get-hit
/// reaction) → S→C 0x0D:
/// type, GUID, 0x13, the path cell, the unit's hit class, life percent
/// (the client shows the life bar change without a hit animation).
pub fn soft_hit_message<X: Pending>(
    v: &mut View<'_, X>,
    game: &Game,
    client: ClientId,
    unit: UnitId,
) {
    let Some(receiver) = game.lists.client(client).and_then(|c| c.player) else {
        return;
    };
    let Some(r) = v.units.get(unit) else {
        return;
    };
    if r.ty != UnitType::Player || r.flags & UNIT_FLAG_SOFT_HIT == 0 {
        return;
    }
    let (ty, guid) = (r.ty as u8, r.guid);
    let Some(pos) =
        v.h.paths
            .as_ref()
            .and_then(|p| p.dynamic(unit))
            .map(|p| p.cell())
    else {
        return;
    };
    let m = crate::path::walk::messages::player_stop(
        ty,
        guid,
        0x13,
        pos.x as u16,
        pos.y as u16,
        v.unit_b0(unit),
        life_percent(v, unit),
    );
    v.h.x.send(receiver, &m);
}

/// One client's message of a [`StopRow`].
fn stop_row_messages<X: Pending>(
    v: &mut View<'_, X>,
    receiver: UnitId,
    unit: UnitId,
    (ty, guid): (u8, u32),
    path: &crate::path::record::DynamicPath,
    row: StopRow,
    own: bool,
) {
    use crate::path::walk::messages::{player_move, player_stop};
    let pos = path.cell();
    let (x, y) = (pos.x as u16, pos.y as u16);
    let b0 = v.unit_b0(unit);
    let life = life_percent(v, unit);
    let (code, b) = match row {
        StopRow::Neutral if own => return,
        StopRow::Neutral => (7, 0),
        StopRow::Stop(c) => (c, b0),
        StopRow::Death => (8, b0),
        StopRow::Knockback => {
            let m = player_move(ty, guid, 0x14, path.target_x, path.target_y, x, y);
            let mut m = m;
            m[11] = b0;
            v.h.x.send(receiver, &m);
            return;
        }
    };
    v.h.x
        .send(receiver, &player_stop(ty, guid, code, x, y, b, life));
    if row == StopRow::Death && own {
        let g = v.stats.unit_total(unit, 175, 0);
        if let Some(m) = crate::wiring::action::vitals_sync::stat_message(175, g) {
            v.h.x.send(receiver, &m);
        }
    }
}

impl<X: Pending> CollisionRooms for PathCtx<'_, X> {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        self.v.h.drlg.subtile_rect(room)
    }
    fn adjacent_count(&self, room: RoomId) -> usize {
        self.v.h.drlg.adjacent_count(room)
    }
    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        self.v.h.drlg.adjacent(room, i)
    }
    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        self.v.h.drlg.grid(room)
    }
    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        self.v.h.drlg.grid_mut(room)
    }
}

impl<X: Pending> PathWorld for PathCtx<'_, X> {
    fn load_path(&self, unit: UnitId) -> Option<DynamicPath> {
        self.v.h.paths.as_ref()?.dynamic(unit).cloned()
    }
    fn store_path(&mut self, unit: UnitId, path: &DynamicPath) {
        if let Some(d) = self.v.h.paths.as_mut().and_then(|p| p.dynamic_mut(unit)) {
            *d = path.clone();
        }
    }
    fn room_in_town(&self, room: RoomId) -> bool {
        self.v.h.drlg.in_town(self.game, room)
    }
    fn remove_footprint(&mut self, unit: UnitId, force: bool) -> bool {
        self.v.path_remove_footprint(unit, force)
    }
    fn add_footprint(&mut self, unit: UnitId) {
        self.v.path_add_footprint(unit);
    }
    fn room_list_remove(&mut self, unit: UnitId, _: RoomId) {
        if let Err(e) = self.game.lists.room_remove(unit) {
            self.v
                .unit_error(crate::units::modes::UnitError::Game(e.into()));
        }
    }
    /// Room list insert (D2RUST's insert also queues the
    /// unit for the room's update list in the same step).
    fn room_list_insert(&mut self, unit: UnitId, room: RoomId) {
        if let Err(e) = self.game.lists.room_insert(unit, room) {
            self.v
                .unit_error(crate::units::modes::UnitError::Game(e.into()));
        }
    }
    fn queue_for_update(&mut self, unit: UnitId) {
        if let Err(e) = self.game.lists.queue_update(unit) {
            self.v
                .unit_error(crate::units::modes::UnitError::Game(e.into()));
        }
    }
    /// The active room's client array.
    fn room_clients(&self, room: RoomId) -> Vec<ClientId> {
        self.v
            .h
            .drlg
            .drlg_room(self.game, room)
            .and_then(|(d, r)| d.active_room(r))
            .map_or_else(Vec::new, |a| a.clients.clone())
    }
    /// Whether the room still exists and belongs to the unit's act.
    fn room_in_unit_act(&self, unit: UnitId, room: RoomId) -> bool {
        let act = self.v.units.get(unit).map(|r| r.act);
        act.is_some() && self.game.lists.room(room).map(|r| r.act) == act
    }
}

impl<X: Pending> WalkUnits for PathCtx<'_, X> {
    fn frame(&self) -> i32 {
        self.game.frame
    }
    fn tick_rate(&self) -> TickRate {
        self.game.tick_rate
    }
    /// S→C bytes to the player's own client (the resync's 0x15,
    /// its position correction).
    fn send_to_client(&mut self, unit: UnitId, bytes: &[u8]) {
        self.v.h.x.send(unit, bytes);
    }
    fn unit_type(&self, unit: UnitId) -> UnitType {
        self.v.units.get(unit).map_or(UnitType::Tile, |r| r.ty)
    }
    fn class(&self, unit: UnitId) -> u32 {
        self.v.units.get(unit).map_or(0, |r| r.class)
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.v.units.get(unit).map_or(0, |r| r.guid)
    }
    fn mode(&self, unit: UnitId) -> u32 {
        self.v.units.get(unit).map_or(0, |r| r.mode)
    }
    /// Finds a unit by type and GUID.
    fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        self.game.lists.find_unit(ty, guid)
    }
    fn position(&self, unit: UnitId) -> Point {
        let (x, y) = self.v.h.path_position(unit);
        Point::new(x, y)
    }
    fn unit_size(&self, unit: UnitId) -> i32 {
        self.v.path_size(unit)
    }
    /// The smallest positive expire of the unit's type-1 timers
    /// (the next timer that is due).
    fn first_type1_expire(&self, unit: UnitId) -> i32 {
        let q = &self.game.timers;
        q.unit_timers(unit)
            .into_iter()
            .filter_map(|t| {
                let (ev, _, _) = q.event(t)?;
                (ev == crate::tick::events::event::END_ANIM).then_some(q.expire(t)?)
            })
            .filter(|&e| e > 0)
            .min()
            .unwrap_or(0)
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.v.stats.has_state(unit, u32::from(state))
    }
    fn state_stat(&self, unit: UnitId, state: u16, stat: u16) -> i32 {
        self.v.state_stat(unit, state, stat).unwrap_or(0)
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.v.stat(unit, stat)
    }
    /// The item's stat ([`Pending::item_stat`], layer 0).
    fn item_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.v.h.x.item_stat(unit, stat, 0)
    }
    /// Base stat add.
    fn add_base_stat(&mut self, unit: UnitId, stat: u16, delta: i32) {
        let v = self.v.stats.unit_base(unit, stat, 0).wrapping_add(delta);
        self.v.set_base(unit, stat, v);
    }
    fn set_base_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.v.set_base(unit, stat, value);
    }
    fn seed(&mut self, unit: UnitId) -> &mut Seed {
        self.v.seed(unit)
    }
    /// The used skill ([`Pending::used_skill`]) and its skills.txt row. The
    /// flags are the used skill entry's
    /// runtime E-flags word, not a skills.txt column.
    fn used_skill(&self, unit: UnitId) -> Option<UsedSkill> {
        let e = self.v.h.used_skill_of(unit)?;
        let row = self.v.h.tables.skills.skills.get(e.skill as usize)?;
        Some(UsedSkill {
            id: e.skill as u16,
            seq_input: i32::from(row.seqinput),
            srvdofunc: i32::from(row.srvdofunc),
            interrupt: row.interrupt,
            skill_flags: self.v.h.entry_flags_of(unit, &e),
        })
    }
    /// Mode set through the unit system.
    fn set_mode(&mut self, unit: UnitId, mode: u32) {
        let r = {
            let mut sim = crate::units::hooks::Sim {
                game: &mut *self.game,
                units: &mut *self.v.units,
                stats: &mut *self.v.stats,
                data: self.v.data,
            };
            crate::units::modes::set_mode(&mut sim, &mut *self.v.h, unit, mode)
        };
        if let Err(e) = r {
            self.v.unit_error(e);
        }
    }
    /// Cancels the unit's pending events of one type.
    fn cancel_events(&mut self, unit: UnitId, ty: u8) {
        self.game.timers.cancel_unit_events(unit, ty, None);
    }
    /// Schedules event 0 (the next movement step) for the unit.
    fn schedule_event0(&mut self, unit: UnitId) {
        if let Err(e) = crate::units::anim::every_tick_movement(self.game, unit) {
            self.v.unit_error(crate::units::modes::UnitError::Game(e));
        }
    }
    /// The start function of a mode other than 2, 3, 6, 19 (player:
    /// the mode system's start table; the neutral
    /// re-entry). Targets are not passed: the unit system's starts take
    /// none.
    ///
    /// TODO: `player_start` also runs the mode request
    /// check hook; [`crate::wiring::action::ActionHooks`] keeps its
    /// default (accept), so only the start function runs. Monster
    /// starts: the monster mode set.
    fn start_other_mode(&mut self, unit: UnitId, mode: u32, _: StartTarget) {
        let player = self.unit_type(unit) == UnitType::Player;
        let r = {
            let mut sim = crate::units::hooks::Sim {
                game: &mut *self.game,
                units: &mut *self.v.units,
                stats: &mut *self.v.stats,
                data: self.v.data,
            };
            if player {
                crate::units::modes::player_start(&mut sim, &mut *self.v.h, unit, mode).map(|_| ())
            } else {
                crate::units::modes::monster_set_mode(&mut sim, &mut *self.v.h, unit, mode)
            }
        };
        if let Err(e) = r {
            self.v.unit_error(e);
        }
    }
    /// The run stat list: a TEMPONLY
    /// list owned by the unit, stat 67 :=
    /// `value`, attached to the unit; the next real mode change frees it
    /// (it is freed with the mode's other temporary lists).
    /// The attach runs only from the animation setup, which
    /// the mode set calls only on a new mode; a run request
    /// while already running (a second 0x03) reaches neither, so the one
    /// run list stays as it is and stat 67 is not written again (read of
    fn attach_run_stats(&mut self, unit: UnitId, value: i32) {
        use crate::path::walk::velocity::STAT_VELOCITYPERCENT as VELOCITYPERCENT;
        use crate::stats::lists::flag;
        let Some((ty, guid)) = self
            .v
            .units
            .get(unit)
            .map(|r| (r.ty.index() as u32, r.guid))
        else {
            return;
        };
        let stats = &*self.v.stats;
        let existing = stats.unit_list(unit).and_then(|root| {
            stats.active_chain(root).into_iter().find(|&l| {
                stats.flags(l) & flag::TEMPONLY != 0 && stats.base(l, VELOCITYPERCENT, 0) != 0
            })
        });
        match existing {
            Some(_) => {}
            None => {
                let l = self.v.stats.alloc(flag::TEMPONLY, 0, ty, guid);
                self.v.set_list_stat(l, VELOCITYPERCENT, value);
                let v = &mut self.v;
                v.stats.attach(&mut *v.h, unit, l, true);
                self.refresh_run_rate(unit);
            }
        }
    }
    fn charstats_velocity(&self, unit: UnitId) -> (i32, i32, i32) {
        self.v.charstats_velocity(unit)
    }
    fn monstats_velocity(&self, unit: UnitId) -> (i32, bool) {
        self.v.monster_velocity(unit)
    }
    fn velocity_base(&self, unit: UnitId) -> i32 {
        self.v.velocity_base(unit)
    }
    /// Whether a monster may be in town (monstats).
    fn monster_can_be_in_town(&self, unit: UnitId) -> bool {
        self.v
            .monster_shape(unit)
            .is_some_and(|m| m.can_be_in_town())
    }
    fn set_unit_flag(&mut self, unit: UnitId, bit: u32) {
        if let Some(r) = self.v.units.get_mut(unit) {
            r.flags |= bit;
        }
    }
    /// Room change: the monster's AI vision record (the room memo the AI scan
    /// uses) := none.
    fn clear_ai_room_memo(&mut self, unit: UnitId) {
        if let Some(m) = self.v.h.monster_data_mut(unit) {
            m.vision = None;
        }
    }
    /// The player of a client record.
    fn client_player(&self, client: ClientId) -> Option<UnitId> {
        self.game.lists.client(client)?.player
    }
    /// Room change: S→C 0x0A to the client's player.
    fn send_unit_removal(&mut self, client: ClientId, unit: UnitId) {
        let (Some(receiver), Some(e)) = (self.client_player(client), self.game.lists.unit(unit))
        else {
            return;
        };
        let m = crate::units::messages::remove_unit(e.ty as u8, e.guid);
        self.v.h.x.send(receiver, &m);
    }
    /// Room change: the add messages to
    /// the client's player.
    fn send_unit_add(&mut self, client: ClientId, unit: UnitId) {
        let Some(receiver) = self.client_player(client) else {
            return;
        };
        self.v.add_messages(self.game, receiver, unit);
    }
    /// [`crate::wiring::path::PathState::history`] (players only).
    fn position_history(&mut self, unit: UnitId) -> Option<&mut PositionHistory> {
        if self.unit_type(unit) != UnitType::Player {
            return None;
        }
        Some(self.v.h.paths.as_mut()?.history.entry(unit).or_default())
    }
}

impl<X: Pending> PathMotion for PathCtx<'_, X> {
    /// Set position `(Q, hint)` at the cell centre, with the room recache
    /// (the unit's room is looked up again).
    fn set_position(&mut self, path: &mut DynamicPath, x: i32, y: i32, hint: Option<RoomId>) {
        let Some(unit) = path.owner else {
            return;
        };
        let t = self.tables();
        Walk { t: &t, c: self }.set_position(
            unit,
            path,
            (to_fp16_center(x), to_fp16_center(y)),
            hint,
        );
    }
    /// Reset the path.
    fn reset(&mut self, path: &mut DynamicPath) {
        let Some(unit) = path.owner else {
            return;
        };
        let t = self.tables();
        Walk { t: &t, c: self }.reset(unit, path);
    }
}
