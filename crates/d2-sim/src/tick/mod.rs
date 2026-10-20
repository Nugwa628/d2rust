//! One server tick: the frame counter, the fixed step order, the room
//! pass, the timer run, the client pass and the periodic steps.
//!
//! The host schedule and the wall-clock, host-only parts (frame-rate
//! statistics, heartbeat) belong to `d2-server` and are not here; the
//! client pass's character save is raised here as
//! [`Game::character_save_due`] and written by `d2-server`. d2-sim owns
//! the step order, the list iteration and the flags; the bodies of the
//! steps that belong to other systems (environment, population, messages,
//! quests, items, AI, ...) are hooks: [`TickHooks`] and, for timer events,
//! [`EventDispatch`].
//!
//! Step order per tick: 0 frame += 1; 1 environment (day/night per act);
//! 2 frame-rate statistics (host only); 3 room pass (spawns, population);
//! 4 timer queue; 5 client pass (load, join, per-client update);
//! 6 room update queues; 7 removal records; then the periodic steps
//! 8 quests, 9 room deactivation, 10 free inactive rooms, 11 expired items.
//! Changing this order changes which tick a message or spawn lands in.

pub mod events;
pub mod rate;
pub mod timer;

pub use rate::TickRate;

use crate::game::Game;
use crate::units::lists::{client_state, ACTS};
use crate::units::{ClientId, RoomId, UnitId};
use timer::{TimerClass, TimerList, TimerRun};

/// Tick length in ms: `1000 / rate`, integer division. Used by the
/// host schedule only.
pub const TICK_LENGTH_MS: u32 = 1000 / crate::TICKS_PER_SECOND;

/// Periods of the periodic steps in frames at the base rate (scaled by
/// `Game::ticks` at a faster one).
pub mod period {
    pub const QUESTS: i32 = 20;
    pub const ROOM_DEACTIVATION: i32 = 12;
    pub const FREE_INACTIVE_ROOMS: i32 = 11;
    pub const EXPIRED_ITEMS: i32 = 1500;
    /// The client pass's character save.
    pub const CHARACTER_SAVE: i32 = 8192;
}

/// Room inactivity counter threshold for step 9: deactivate when > 10.
pub const ROOM_DEACTIVATION_THRESHOLD: u32 = 10;

/// Whether a periodic step with `period` runs at `frame` (the signed
/// remainder of the frame by the period is 0).
pub const fn is_due(frame: i32, period: i32) -> bool {
    timer::frame_mod(frame, period) == 0
}

/// Runs one timer event: the timer's callback, or its class's default
/// handler (data in [`events`]). What each event does per unit kind
/// belongs to the unit systems.
///
/// The handler may schedule and cancel timers and remove units through
/// `game`; the queue's cursor rules keep the run consistent.
pub trait EventDispatch {
    fn run_event(&mut self, game: &mut Game, run: &TimerRun);
}

/// Bodies of the tick steps that belong to other systems. Each default
/// does nothing: a world without that system, not a claimed behaviour.
/// d2-sim calls them in the tick's step order and owns the lists and
/// flags around them.
#[allow(unused_variables)]
pub trait TickHooks: EventDispatch {
    /// Step 1: advance act `act`'s day/night cycle with `A` = `act`; true when the
    /// index or type changed or the hour moved more than 16 degrees since
    /// the last report.
    fn advance_environment(&mut self, game: &mut Game, act: u8) -> bool {
        false
    }

    /// Step 1, per client when act `act`'s cycle changed: refresh the
    /// player's items, then message 0x53 if the client is
    /// in game (state 4) in that act.
    fn environment_changed(&mut self, game: &mut Game, act: u8, client: ClientId) {}

    /// Step 3: ambient spawns of an active room.
    fn ambient_spawns(&mut self, game: &mut Game, room: RoomId) {}

    /// Step 3: preset units.
    fn spawn_presets(&mut self, game: &mut Game, room: RoomId) {}

    /// Step 3: restore inactive units.
    fn restore_inactive_units(&mut self, game: &mut Game, room: RoomId) {}

    /// Step 3: object population.
    fn populate_objects(&mut self, game: &mut Game, room: RoomId) {}

    /// Step 3: monster population.
    fn populate_monsters(&mut self, game: &mut Game, room: RoomId) {}

    /// Step 5: the client's room is ready.
    fn client_room_ready(&mut self, game: &mut Game, client: ClientId) -> bool {
        false
    }

    /// Step 5: message 4 to the client (before its state becomes 4).
    fn send_load_complete(&mut self, game: &mut Game, client: ClientId) {}

    /// Step 5: inventory refresh after the state change.
    fn refresh_inventory(&mut self, game: &mut Game, client: ClientId) {}

    /// Step 5, joining clients: the join sequence (the join messages, the
    /// host callback, message 0x5A to all).
    fn join_sequence(&mut self, game: &mut Game, client: ClientId) {}

    /// Per-client update: removal messages for units
    /// deleted in the adjacent rooms.
    fn send_removed_units(&mut self, game: &mut Game, client: ClientId) {}

    /// Per-client update: one unit's update message,
    /// for each unit of each adjacent room's update queue.
    fn send_unit_update(&mut self, game: &mut Game, client: ClientId, unit: UnitId) {}
    /// After the walk of one room's update queue (host marks).
    fn update_room_end(&mut self, game: &mut Game, client: ClientId, room: RoomId) {}

    /// Per-client update: player stat-change messages; then, only when the
    /// player's flag-ex bit 21 is set, the inventory refresh (skipped
    /// otherwise).
    fn client_update_messages(&mut self, game: &mut Game, client: ClientId) {}

    /// Per-client update: the player's room differs from the
    /// client's: level change and room switch.
    fn client_level_change(&mut self, game: &mut Game, client: ClientId) {}

    /// Per-client update: arena sync.
    fn arena_sync(&mut self, game: &mut Game, client: ClientId) {}

    /// Step 6: per unit of an update queue.
    fn unit_update(&mut self, game: &mut Game, unit: UnitId) {}

    /// Step 6, last: clears an arena flag bit.
    fn clear_arena_flag(&mut self, game: &mut Game) {}

    /// Step 7: free an active room's unit-removal records.
    fn free_removal_records(&mut self, game: &mut Game, room: RoomId) {}

    /// Step 8: quest updater.
    fn update_quests(&mut self, game: &mut Game) {}

    /// Step 9: the room's inactivity counter (raised by the world that
    /// owns the rooms).
    fn room_inactivity(&mut self, game: &mut Game, room: RoomId) -> u32 {
        0
    }

    /// Step 9: the act allows removing the room.
    fn act_allows_room_removal(&mut self, game: &mut Game, act: u8, room: RoomId) -> bool {
        false
    }

    /// Step 9: compress a unit to inactive storage.
    fn compress_unit(&mut self, game: &mut Game, unit: UnitId) {}

    /// Step 9, after the room is unlinked from the act list: the rest of
    /// the room's removal, which belongs to the DRLG (the neighbours'
    /// adjacency arrays, the record free, the tiles).
    fn room_deactivated(&mut self, game: &mut Game, act: u8, room: RoomId) {}

    /// Step 10: free inactive rooms of an act.
    fn free_inactive_rooms(&mut self, game: &mut Game, act: u8) {}

    /// Step 11: delete inactive items of an act.
    fn delete_inactive_items(&mut self, game: &mut Game, act: u8) {}

    /// Step 11: expired inactive-unit item nodes of an act.
    fn expire_inactive_unit_items(&mut self, game: &mut Game, act: u8) {}

    /// The end of the tick, after every step: a mod game's periodic event
    /// hook (`crate::rules::EventHooks::periodic`) runs here.
    fn end_of_tick(&mut self, game: &mut Game) {}
}

/// One tick: frame += 1, then steps 1–11 in order.
pub fn tick<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    tick_through_timers(game, hooks);
    tick_from_client_pass(game, hooks);
}

/// The first half of [`tick`]: frame += 1 and steps 1–4, ending with the
/// timer queue. A host whose unit work runs inside step 4 but outside
/// [`EventDispatch`] runs it between this and [`tick_from_client_pass`],
/// so its messages reach the same tick's client pass.
pub fn tick_through_timers<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    tick_through_rooms(game, hooks);
    run_timer_events(game, hooks);
}

/// The frame advance and steps 1–3 of [`tick_through_timers`]. A host may
/// time this separately from [`run_timer_events`], without introducing a
/// wall clock into simulation rules. Both calls must run in that order,
/// with the same game and hooks, before the client pass.
pub fn tick_through_rooms<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    // Step 0: the game's debug trap switch is never set in play and is
    // not modelled.
    game.frame = game.frame.wrapping_add(1);
    #[cfg(feature = "rng-trace")]
    crate::debug::rng_trace::mark_frame(game.frame);
    environment(game, hooks);
    // Step 2 (frame-rate statistics) is wall-clock only.
    room_pass(game, hooks);
}

/// The second half of [`tick`]: steps 5–11, from the client pass to the
/// periodic steps.
pub fn tick_from_client_pass<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    client_pass(game, hooks);
    room_update_queues(game, hooks);
    removal_records(game, hooks);
    let frame = game.frame;
    if is_due(frame, game.ticks(period::QUESTS)) {
        hooks.update_quests(game);
    }
    if is_due(frame, game.ticks(period::ROOM_DEACTIVATION)) {
        room_deactivation(game, hooks);
    }
    if is_due(frame, game.ticks(period::FREE_INACTIVE_ROOMS)) {
        for act in 0..ACTS as u8 {
            if game.lists.act(act).is_some() {
                hooks.free_inactive_rooms(game, act);
            }
        }
    }
    if is_due(frame, game.ticks(period::EXPIRED_ITEMS)) {
        for act in 0..ACTS as u8 {
            if game.lists.act(act).is_some() {
                hooks.delete_inactive_items(game, act);
                hooks.expire_inactive_unit_items(game, act);
            }
        }
    }
    hooks.end_of_tick(game);
}

/// Whether the day clock advances this tick: it counts base ticks (the
/// ticks S→C 0x53 carries), so at a faster rate it advances on the ticks
/// that start a base tick, 25 a second. Every tick at the base rate.
pub fn environment_due(game: &Game) -> bool {
    game.tick_rate.share(1, game.frame) != 0
}

/// Step 1.
fn environment<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    if !environment_due(game) {
        return;
    }
    for act in 0..ACTS as u8 {
        if game.lists.act(act).is_none() || !hooks.advance_environment(game, act) {
            continue;
        }
        let mut cur = game.lists.client_first();
        while let Some(c) = cur {
            cur = game.lists.client_next(c);
            hooks.environment_changed(game, act, c);
        }
    }
}

/// Step 3, the room pass.
fn room_pass<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    for act in 0..ACTS as u8 {
        if !game.lists.act(act).is_some_and(|a| a.pending_rooms) {
            continue;
        }
        let mut cur = game.lists.room_first(act);
        while let Some(room) = cur {
            room_body(game, hooks, room);
            // Next read after the body.
            cur = game.lists.room_next(room);
        }
        if let Some(a) = game.lists.act_mut(act) {
            a.pending_rooms = false;
        }
    }
}

/// Off-tick population: the room pass body (rules 1–3) for one room,
/// without the act flag test and without clearing the act's pending flag.
/// Called by the portal and arrival paths of monster population inside
/// whichever step runs them; the room stays in its act list, so the next
/// step 3 visits it again (one more ambient call, then nothing).
pub fn populate_room<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H, room: RoomId) {
    room_body(game, hooks, room);
}

/// The room pass rules 1–3 for one room.
fn room_body<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H, room: RoomId) {
    hooks.ambient_spawns(game, room);
    let (populated, active) = match game.lists.room(room) {
        Some(r) => (r.populated, r.units_active),
        None => (true, true),
    };
    if !populated {
        hooks.spawn_presets(game, room);
        hooks.restore_inactive_units(game, room);
        hooks.populate_objects(game, room);
        hooks.populate_monsters(game, room);
        if let Some(r) = game.lists.room_mut(room) {
            r.populated = true;
            r.units_active = true;
        }
    } else if !active {
        hooks.restore_inactive_units(game, room);
        if let Some(r) = game.lists.room_mut(room) {
            r.units_active = true;
        }
    }
}

/// Step 4: run the timer queue.
pub fn run_timer_events<D: EventDispatch + ?Sized>(game: &mut Game, dispatch: &mut D) {
    let frame = game.frame;
    game.timers.begin_run(frame);
    for class in TimerClass::RUN_ORDER {
        for list in [TimerList::EveryTick, TimerList::Due] {
            game.timers.begin_list(class, list);
            while let Some(run) = game.timers.next_run(list, frame) {
                dispatch.run_event(game, &run);
                game.timers.finish_run(&run);
            }
        }
    }
}

/// Step 5, the client pass. The arena assert and the
/// heartbeat are host-only. The 8192-frame save (rule 3) runs in
/// single player too (no heartbeat drop is modelled, so only the frame
/// decides): it is raised as [`Game::character_save_due`], before the
/// per-client loop, for the host's character storage to write.
fn client_pass<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    if is_due(game.frame, game.ticks(period::CHARACTER_SAVE)) {
        game.character_save_due = true;
    }
    let mut cur = game.lists.client_first();
    while let Some(c) = cur {
        // Next saved before the body.
        cur = game.lists.client_next(c);
        let Some(state) = game.lists.client(c).map(|e| e.state) else {
            continue;
        };
        match state {
            client_state::JOINING | client_state::CHANGING_ACT => {
                client_update(game, hooks, c);
                if hooks.client_room_ready(game, c) {
                    hooks.send_load_complete(game, c);
                    if let Some(e) = game.lists.client_mut(c) {
                        e.state = client_state::IN_GAME;
                    }
                    hooks.refresh_inventory(game, c);
                    if state == client_state::JOINING {
                        hooks.join_sequence(game, c);
                    }
                }
            }
            client_state::IN_GAME => client_update(game, hooks, c),
            _ => {}
        }
    }
}

/// The per-client update.
fn client_update<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H, client: ClientId) {
    hooks.send_removed_units(game, client);
    let mut adjacent = game
        .lists
        .client(client)
        .and_then(|e| e.room)
        .and_then(|r| game.lists.room(r))
        .map(|r| r.adjacent.clone())
        .unwrap_or_default();
    // A view wider than 4:3: the rooms beyond the adjacency array the
    // client sees (still active ones only).
    if let Some(e) = game.lists.client(client) {
        for &r in &e.view_rooms {
            if !adjacent.contains(&r) && game.lists.room(r).is_some() {
                adjacent.push(r);
            }
        }
    }
    for room in adjacent {
        let mut cur = game.lists.update_first(room);
        while let Some(u) = cur {
            // Next saved before the body.
            cur = game.lists.update_next(u);
            hooks.send_unit_update(game, client, u);
        }
        hooks.update_room_end(game, client, room);
    }
    hooks.client_update_messages(game, client);
    let Some(e) = game.lists.client_mut(client) else {
        return;
    };
    e.update_count = e.update_count.wrapping_add(1);
    let (player, client_room) = (e.player, e.room);
    let player_room = player
        .and_then(|p| game.lists.unit(p))
        .and_then(|u| u.room());
    if player_room != client_room {
        hooks.client_level_change(game, client);
    }
    hooks.arena_sync(game, client);
    if let Some(p) = game.lists.client(client).and_then(|e| e.player) {
        if game.lists.unit(p).is_some() {
            // Cannot fail: the unit exists.
            let _ = game.lists.queue_update(p);
        }
    }
}

/// Step 6: walk and clear the update queues of acts with
/// pending updates.
fn room_update_queues<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    for act in 0..ACTS as u8 {
        if !game.lists.act(act).is_some_and(|a| a.pending_updates) {
            continue;
        }
        let mut room_cur = game.lists.room_first(act);
        while let Some(room) = room_cur {
            let mut cur = game.lists.update_first(room);
            while let Some(u) = cur {
                // The next link is saved before the body, as for the
                // client-update walk, so the body may change the queue.
                cur = game.lists.update_next(u);
                hooks.unit_update(game, u);
            }
            // Cannot fail: the room is in the act list.
            let _ = game.lists.clear_update_queue(room);
            room_cur = game.lists.room_next(room);
        }
        if let Some(a) = game.lists.act_mut(act) {
            a.pending_updates = false;
        }
    }
    hooks.clear_arena_flag(game);
}

/// Step 7.
fn removal_records<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    for act in 0..ACTS as u8 {
        if !game.lists.act(act).is_some_and(|a| a.pending_removals) {
            continue;
        }
        let mut cur = game.lists.room_first(act);
        while let Some(room) = cur {
            hooks.free_removal_records(game, room);
            cur = game.lists.room_next(room);
        }
        if let Some(a) = game.lists.act_mut(act) {
            a.pending_removals = false;
        }
    }
}

/// Step 9: deactivate rooms inactive for more than 10
/// counts where the act allows it.
fn room_deactivation<H: TickHooks + ?Sized>(game: &mut Game, hooks: &mut H) {
    for act in 0..ACTS as u8 {
        if game.lists.act(act).is_none() {
            continue;
        }
        let mut cur = game.lists.room_first(act);
        while let Some(room) = cur {
            // Next saved before the body.
            cur = game.lists.room_next(room);
            if hooks.room_inactivity(game, room) > ROOM_DEACTIVATION_THRESHOLD
                && hooks.act_allows_room_removal(game, act, room)
            {
                let mut unit_cur = game.lists.room_unit_first(room);
                while let Some(u) = unit_cur {
                    unit_cur = game.lists.room_unit_next(u);
                    hooks.compress_unit(game, u);
                }
                // Removes the room from the act list; what
                // else it frees belongs to the DRLG.
                let _ = game.lists.deactivate_room(room);
                hooks.room_deactivated(game, act, room);
            }
        }
    }
}

#[cfg(test)]
mod gaps_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_fsim;
#[cfg(test)]
mod tests_lsim;
