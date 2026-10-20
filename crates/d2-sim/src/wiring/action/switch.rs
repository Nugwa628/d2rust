//! The client room switch on the action wiring: the DRLG bookkeeping
//! ([`super::DrlgWorld::client_switches_room`]), then, for every room the
//! client joined (new array order), S->C 0x07, the AI wake-up of its monsters
//! when the client is the room's only one, and the add messages of its
//! units; then, for every room it left (old array order), S->C 0x0A for its
//! units, S->C 0x08, and the player update when the room was the client's.
//! Callers: the per-client update ([`super::ActionSim`]'s
//! `client_level_change`) and game entry (through `crate::wiring::path::place`).
//!
//! Monster add messages: [`super::monster_add`].
//!
//! Also the room-ready test and the session state the join reads
//! ([`SessionState`]).
//!
//! Every message goes to the client's player ([`Pending::send`]).
//!
//! Not sent, because the rules for them are not implemented here:
//! - item 0x9C (the item world is not reachable from the action wiring);
//!   missile 0x73 (`View::missile_add`) is sent;
//! - player part B for another player (multiplayer only) and the inventory
//!   messages; the corpse 0x74 is sent ([`corpse_assign`]: flag, owner GUID,
//!   corpse GUID, -1 for a missing unit);
//! - the portal-flag record update.

use std::collections::BTreeMap;

use crate::game::Game;
use crate::monsters::ai;
use crate::units::messages::{self, SendStat, SentState};
use crate::units::{ClientId, RoomId, UnitId, UnitType};

use super::rooms::RoomSwitch;
use super::{Pending, View, WiringError};

/// The session facts of the players a client join brings in (the save): read
/// by the add messages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionState {
    /// Player data names (0x59 bytes 6..22), zero-padded.
    pub names: BTreeMap<UnitId, [u8; 16]>,
    /// The overhead records (written by the C->S 0x14 handler through
    /// [`View::replace_overhead`]): text and language byte. The record is
    /// live while the unit's `hover` frame is set.
    pub overheads: BTreeMap<UnitId, (Vec<u8>, u8)>,
    /// The player's item messages of the join (0x9C, 0x9D), queued by the
    /// loader and sent once by the join after the stat messages.
    pub join_items: BTreeMap<UnitId, Vec<Vec<u8>>>,
    /// The quest entry's messages of the join (0x5E, 0x28, 0x29, 0x89),
    /// queued by the loader and sent by the join after the loader's other
    /// messages.
    pub join_quest: BTreeMap<UnitId, Vec<Vec<u8>>>,
    /// The joiner's hireling add messages (0x81 / 0x7A), sent by the join
    /// right after its own 0x5B.
    pub join_pets: BTreeMap<UnitId, Vec<Vec<u8>>>,
}

/// S->C 0x59 AssignPlayer (26 bytes): GUID u32@1, class u8@5, name 16 bytes @6, x u16@0x16, y u16@0x18.
pub fn assign_player(guid: u32, class: u8, name: &[u8; 16], x: u16, y: u16) -> [u8; 26] {
    let mut b = [0u8; 26];
    b[0] = 0x59;
    b[1..5].copy_from_slice(&guid.to_le_bytes());
    b[5] = class;
    b[6..22].copy_from_slice(name);
    b[22..24].copy_from_slice(&x.to_le_bytes());
    b[24..26].copy_from_slice(&y.to_le_bytes());
    b
}

/// S->C 0x74 PlayerCorpseAssign (10 bytes: flag u8@1, player u32@2,
/// corpse u32@6) for the corpse `guid`. The flag is always 1, so the bytes
/// are `74 01 <owner GUID> <corpse GUID>`.
pub fn corpse_assign(owner: u32, corpse: u32) -> [u8; 10] {
    let mut b = [0u8; 10];
    b[0] = 0x74;
    b[1] = 1;
    b[2..6].copy_from_slice(&owner.to_le_bytes());
    b[6..10].copy_from_slice(&corpse.to_le_bytes());
    b
}

impl<X: Pending> View<'_, X> {
    /// The room switch of `client` to `new` (module docs). Nothing when the
    /// client is unknown or `new` is its room; a switch between acts (the
    /// act change) is not handled here: the client keeps its room.
    pub fn room_switch(&mut self, game: &mut Game, client: ClientId, new: Option<RoomId>) {
        let Some(e) = game.lists.client(client) else {
            return;
        };
        let (old, player) = (e.room, e.player);
        if old == new {
            return;
        }
        let act_of = |r: Option<RoomId>| r.and_then(|r| game.lists.room(r)).map(|r| r.act);
        let act = match (act_of(old), act_of(new)) {
            (Some(a), Some(b)) if a != b => return,
            (Some(a), _) | (None, Some(a)) => a,
            (None, None) => return,
        };
        let switch = self
            .h
            .drlg
            .client_switches_room(&mut game.lists, act, client, old, new);
        let mut view = Vec::new();
        match switch {
            Ok(s) => {
                if let Some(player) = player {
                    self.switch_messages(game, client, player, old, &s);
                }
                view = s.view;
            }
            Err(e) => self.h.errors.push(e),
        }
        if let Some(e) = game.lists.client_mut(client) {
            e.room = new;
            e.view_rooms = view;
        }
    }

    /// The joins and leaves of one switch, to `player`'s client.
    fn switch_messages(
        &mut self,
        game: &mut Game,
        client: ClientId,
        player: UnitId,
        old: Option<RoomId>,
        s: &RoomSwitch,
    ) {
        // Joins.
        for r in &s.joined {
            let reveal = crate::wiring::path::place::map_reveal(
                r.tile_x as u16,
                r.tile_y as u16,
                r.level as u8,
            );
            self.h.x.send(player, &reveal);
            if r.clients <= 1 {
                self.wake_room_monsters(game, r.room);
            }
            for u in game.lists.room_units(r.room) {
                if u != player {
                    self.add_messages(game, player, u);
                }
            }
        }
        // Leaves.
        for r in &s.left {
            for u in game.lists.room_units(r.room) {
                let Some(e) = game.lists.unit(u) else {
                    continue;
                };
                if e.ty != UnitType::Missile {
                    let msg = messages::remove_unit(e.ty as u8, e.guid);
                    self.h.x.send(player, &msg);
                }
            }
            // No client left in the room: every monster in it has its AI
            // think (type 2) and stat regeneration (type 3) events
            // cancelled, so town NPCs stop when the player warps out (the
            // act-1 town NPCs never think again after the act change).
            if r.clients == 0 {
                for u in game.lists.room_units(r.room) {
                    if game
                        .lists
                        .unit(u)
                        .is_some_and(|e| e.ty == UnitType::Monster)
                    {
                        game.timers.cancel_unit_events(u, 2, None);
                        game.timers.cancel_unit_events(u, 3, None);
                    }
                }
            }
            let hide = messages::map_hide(r.tile_x as u16, r.tile_y as u16, r.level as u8);
            self.h.x.send(player, &hide);
            // The player update: the path part, then the event records,
            // the state messages and the stat sends, as the per-client walk
            // runs them. The `just_portaled` 0xA7 is sent here, after the
            // 0x15 and the sound, before the next room's 0x08.
            if Some(r.room) == old {
                if self.h.paths.is_some() {
                    crate::wiring::path::walk::update_messages(self, &*game, client, player);
                }
                self.send_event_records(game, player, player);
                if self.h.paths.is_some() {
                    crate::wiring::path::walk::sound_mark(self, &*game, client, player);
                    crate::wiring::path::walk::soft_hit_message(self, &*game, client, player);
                }
                self.state_change_messages(player, player);
                self.player_stat_sends(player, player);
                // Sent: the room clean-up may clear the state-changed bits
                // (as after the per-client walk's player update).
                if self.h.defer_player_tail && !self.h.player_state_done.contains(&player) {
                    self.h.player_state_done.push(player);
                }
            }
        }
    }

    /// Every monster of the room gets its think restarted
    /// ([`ai::client_entered_room`]). No AI store: nothing.
    fn wake_room_monsters(&mut self, game: &mut Game, room: RoomId) {
        self.with_ai(game, |game, cx| ai::client_entered_room(game, cx, room));
    }

    /// The think restart of one monster ([`ai::update_ai_callback`]). No AI store: nothing.
    pub(super) fn think_restart(&mut self, game: &mut Game, unit: UnitId) {
        self.with_ai(game, |game, cx| ai::update_ai_callback(game, cx, unit));
    }

    /// Runs `f` with the lent AI store on this view. No AI store: nothing.
    fn with_ai(
        &mut self,
        game: &mut Game,
        f: impl FnOnce(&mut Game, &mut ai::Ctx<'_, View<'_, X>>),
    ) {
        let Some(mut store) = self.h.ai.take() else {
            return;
        };
        let t = self.h.tables.clone();
        let info = self.h.ai_info;
        {
            let mut v = View::of(&mut *self.units, &mut *self.stats, self.data, &mut *self.h);
            let mut cx = ai::Ctx {
                tables: ai::AiTables {
                    monstats: &t.combat.monstats,
                    monstats2: &t.combat.monstats2,
                    levels: &t.levels,
                    skill_modes: &t.skill_modes,
                    skills: &t.skills.skills,
                    missiles: &t.skills.missiles,
                },
                info,
                store: &mut store,
                world: &mut v,
            };
            f(game, &mut cx);
        }
        self.h.ai = Some(store);
    }

    /// The add messages of `unit` to the client of `receiver` (module docs
    /// for what is not sent).
    pub fn add_messages(&mut self, game: &Game, receiver: UnitId, unit: UnitId) {
        let Some(e) = game.lists.unit(unit) else {
            return;
        };
        let (ty, guid) = (e.ty, e.guid);
        let Some(r) = self.units.get(unit) else {
            return;
        };
        let (class, mode) = (r.class, r.mode);
        let (x, y) = self.h.path_position(unit);
        match ty {
            UnitType::Player => {
                // A corpse carries its owner's name in its 0x59.
                let named = self
                    .h
                    .death
                    .owners
                    .get(&unit)
                    .and_then(|&o| game.lists.find_unit(UnitType::Player, o))
                    .unwrap_or(unit);
                let name = self
                    .h
                    .session
                    .names
                    .get(&named)
                    .copied()
                    .unwrap_or_default();
                let m = assign_player(guid, class as u8, &name, x as u16, y as u16);
                self.h.x.send(receiver, &m);
                // The assign message is followed by the party info.
                let level = self.stats.unit_total(unit, 12, 0) as u16;
                self.h.x.send(
                    receiver,
                    &messages::player_party_info(guid, level, receiver == unit),
                );
                self.player_part_b(game, receiver, unit);
            }
            UnitType::Object => {
                self.object_add(game, receiver, (unit, guid), (class, mode), (x, y))
            }
            UnitType::Tile => {
                let m = messages::assign_warp(ty as u8, guid, class as u8, x as u16, y as u16);
                self.h.x.send(receiver, &m);
            }
            UnitType::Monster => self.monster_add(game, receiver, unit),
            UnitType::Missile => self.missile_add(game, receiver, unit),
            // The host answers the mark with the ground update. An item
            // still carrying unit flag 0x10 (not yet announced) sends
            // nothing here; the flag is read now, before the room clean-up
            // clears it.
            UnitType::Item => {
                let announced = self
                    .units
                    .get(unit)
                    .is_some_and(|r| r.flags & crate::units::record::flags::SEED_SET == 0);
                if self.h.item_marks && announced {
                    let mut m = [super::JOIN_ITEM_MARK; 5];
                    m[1..].copy_from_slice(&guid.to_le_bytes());
                    self.h.x.send(receiver, &m);
                }
            }
        }
    }

    /// Sends the client its view again after a resume (contracts 3.7: the
    /// resumed client clears its bridge world and the server sends the game
    /// state again). The player unit, its position and the game state stay
    /// as they are: this sends, for the client's room and its view extras,
    /// S→C 0x07 and the add messages of every unit there, then its own add
    /// messages with its current position and the player update the room
    /// switch sends for a player's own room. Nothing is loaded from the
    /// character file and no unit is moved.
    pub fn resend_client(&mut self, game: &mut Game, client: ClientId) {
        let Some((player, room, view_rooms)) = game
            .lists
            .client(client)
            .and_then(|e| Some((e.player?, e.room?, e.view_rooms.clone())))
        else {
            return;
        };
        let mut rooms = Vec::with_capacity(1 + view_rooms.len());
        rooms.push(room);
        for r in view_rooms {
            if !rooms.contains(&r) {
                rooms.push(r);
            }
        }
        for r in rooms {
            // The room's 0x07 fields, read before the send borrows the sink.
            let info = self.h.drlg.drlg_room(game, r).map(|(d, id)| {
                let entry = d.room(id);
                (entry.rect.x, entry.rect.y, d.level(entry.level).id)
            });
            let Some((tile_x, tile_y, level)) = info else {
                continue;
            };
            let reveal =
                crate::wiring::path::place::map_reveal(tile_x as u16, tile_y as u16, level as u8);
            self.h.x.send(player, &reveal);
            for u in game.lists.room_units(r) {
                if u == player {
                    continue;
                }
                // Missiles are not announced on a room join either.
                if self.units.get(u).is_some_and(|e| e.ty == UnitType::Missile) {
                    continue;
                }
                self.add_messages(game, player, u);
            }
        }
        // The player's own add messages carry its current position, then
        // the update tail the room switch sends for the player's room.
        self.add_messages(game, player, player);
        if self.h.paths.is_some() {
            crate::wiring::path::walk::update_messages(self, game, client, player);
        }
        self.send_event_records(game, player, player);
        if self.h.paths.is_some() {
            crate::wiring::path::walk::sound_mark(self, game, client, player);
            crate::wiring::path::walk::soft_hit_message(self, game, client, player);
        }
        self.state_change_messages(player, player);
        self.player_stat_sends(player, player);
    }

    /// The missile add message 0x73: only for a `ClientSend` row and an
    /// existing owner; sent for any path velocity. Position is the path's
    /// cells (`precise >> 16`); the first point is the target while the
    /// velocity is non-zero, else (0, 0).
    fn missile_add(&mut self, game: &Game, receiver: UnitId, unit: UnitId) {
        let Some(d) = self.h.missiles.as_ref().and_then(|s| s.get(unit)).cloned() else {
            return;
        };
        if !self
            .h
            .tables
            .missiles
            .get(usize::from(d.class))
            .is_some_and(|r| r.clientsend)
        {
            return;
        }
        let Some(owner) = d.owner else {
            return;
        };
        if game.lists.find_unit(owner.ty, owner.guid).is_none() {
            return;
        }
        let Some(p) = self.h.paths.as_ref().and_then(|p| p.dynamic(unit)) else {
            return;
        };
        let first = if p.velocity != 0 {
            (u32::from(p.target_x), u32::from(p.target_y))
        } else {
            (0, 0)
        };
        let pierce = self.stats.unit_total(unit, 328, 0) as u8;
        let m = messages::client_missile(
            d.class,
            (p.precise_x >> 16, p.precise_y >> 16),
            first,
            d.current as u16,
            (owner.ty as u8, owner.guid),
            d.level as u8,
            pierce,
        );
        self.h.x.send(receiver, &m);
    }

    /// Player part B as far as it is implemented: the unit's states (0xAA),
    /// its pending event records ([`View::send_event_records`]) and the
    /// overhead text.
    pub fn player_part_b(&mut self, game: &Game, receiver: UnitId, unit: UnitId) {
        let Some(e) = game.lists.unit(unit) else {
            return;
        };
        let (ty, guid) = (e.ty as u8, e.guid);
        // The unit's five base stats go to every other player's client.
        if unit != receiver {
            for s in [67u8, 68, 12, 0, 2] {
                let v = self.stats.unit_base(unit, u16::from(s), 0);
                self.h
                    .x
                    .send(receiver, &messages::stat_update(guid, s, v as u32));
            }
        }
        if self.is_player_corpse(unit) {
            let owner = self
                .h
                .death
                .owners
                .get(&unit)
                .copied()
                .or_else(|| self.h.x.corpse_owner_guid(unit))
                .unwrap_or(guid);
            self.h.x.send(receiver, &corpse_assign(owner, guid));
        }
        let states = self.unit_states_message(ty, guid, unit);
        self.h.x.send(receiver, &states);
        // The mode's update function.
        crate::wiring::path::walk::mode_update_messages(self, game, receiver, unit);
        self.send_event_records(game, receiver, unit);
        self.overhead_message(receiver, unit, ty, guid);
    }

    /// The corpse test of part B: the unit is dead and has state 7
    /// (`playerbody`); a corpse this wiring allocated counts as having
    /// state 7 (as the take-back, [`super::death`]).
    fn is_player_corpse(&self, unit: UnitId) -> bool {
        self.units.get(unit).is_some_and(|r| r.is_dead())
            && (self.stats.has_state(unit, super::death::STATE_PLAYERBODY)
                || self.h.death.owners.contains_key(&unit))
    }

    /// The overhead message: a unit with no overhead record gets S->C 0x76.
    /// Else, unless the unit is a player the receiver relates to (relation
    /// flag 4 from the receiver, or 2 toward it), the overhead chat 0x26
    /// form 5 with the record's text (the record [`View::replace_overhead`]
    /// kept, else [`Pending::overhead_record`]; none: nothing).
    pub(super) fn overhead_message(&mut self, receiver: UnitId, unit: UnitId, ty: u8, guid: u32) {
        let Some(r) = self.units.get(unit) else {
            return;
        };
        if r.hover.is_none() {
            self.h.x.send(receiver, &messages::unit_ref(0x76, ty, guid));
            return;
        }
        if r.ty == UnitType::Player
            && (self.h.x.player_relation(receiver, unit, 4)
                || self.h.x.player_relation(unit, receiver, 2))
        {
            return;
        }
        let record = self.h.session.overheads.get(&unit).cloned();
        if let Some((text, byte8)) = record.or_else(|| self.h.x.overhead_record(unit)) {
            self.h
                .x
                .send(receiver, &messages::overhead_chat(byte8, ty, guid, &text));
        }
    }

    /// Replaces the unit's overhead record: the timeout frame
    /// (`UnitRecord::hover`, read by event 6) is set to `end` and the record
    /// (`text`, language byte `byte8`) kept for the overhead 0x26; then
    /// [`Pending::replace_overhead`].
    pub fn replace_overhead(&mut self, unit: UnitId, text: &[u8], byte8: u8, end: i32) {
        if let Some(r) = self.units.get_mut(unit) {
            r.hover = Some(end);
        }
        self.h
            .session
            .overheads
            .insert(unit, (text.to_vec(), byte8));
        self.h.x.replace_overhead(unit, text, byte8, end);
    }

    /// S->C 0xAA of `unit`: its states in ascending order, filtered (below
    /// the `states` count, no `nosend` bit), each with its stat list (the
    /// base array in list order).
    /// The `nosend` bits and the itemstatcost send columns come from
    /// [`super::ActionHooks::bodies`]; without them no state is `nosend`
    /// and no stat has a row.
    pub fn unit_states_message(&self, unit_type: u8, guid: u32, unit: UnitId) -> Vec<u8> {
        let count = self.stats.data().states.count();
        let bodies = self.h.bodies.as_deref();
        let nosend = |s: usize| {
            bodies
                .and_then(|b| b.state_nosend.get(s))
                .copied()
                .unwrap_or(false)
        };
        let mut states = Vec::new();
        if let Some((bits, _)) = self.stats.state_bits(unit) {
            for (w, word) in bits.iter().enumerate() {
                for b in 0..32 {
                    let s = w * 32 + b;
                    if word & (1 << b) == 0 || s >= count || nosend(s) {
                        continue;
                    }
                    let entries = self.state_list(unit, s as u16).map(|l| {
                        self.stats
                            .base_entries(l)
                            .into_iter()
                            .map(|(k, v)| (k as u32 as u16, crate::stats::key_stat(k), v))
                            .collect()
                    });
                    states.push(SentState {
                        state: s as u16,
                        entries,
                    });
                }
            }
        }
        messages::unit_states(unit_type, guid, &states, |id| {
            bodies
                .and_then(|b| b.stat(i32::from(id)))
                .map(|r| SendStat {
                    bits: r.send_bits,
                    param_bits: r.send_param_bits,
                    signed: r.signed,
                })
        })
    }

    /// Object part A: S->C 0x51 (GUID, class, x, y, mode, the object
    /// data's interact byte), then the portal message 0x60 for a `SubClass`
    /// bit 2 row.
    ///
    /// A game without the object control (`ActionHooks::objects` `None`, no
    /// object data) sends interact 0, as its 0x03 sends an object seed of 0.
    /// Class 59 then sends 0x82 from [`Pending::portal_owner`].
    fn object_add(
        &mut self,
        game: &Game,
        receiver: UnitId,
        (unit, guid): (UnitId, u32),
        (class, mode): (u32, u32),
        (x, y): (i32, i32),
    ) {
        let st = self.h.objects.as_ref();
        let data = st.and_then(|s| s.control.data.get(&unit));
        let interact = data.map_or(0, |d| d.interact);
        let portal = st
            .zip(data)
            .filter(|(s, d)| s.tables.object(d.class).is_ok_and(|o| o.subclass & 4 != 0))
            .map(|(_, d)| crate::world::objects::portal_message(d));
        let m =
            messages::assign_object(guid, class as u16, x as u16, y as u16, mode as u8, interact);
        self.h.x.send(receiver, &m);
        if let Some(b) = portal {
            self.h.x.send(receiver, &b);
        }
        if class == 59 {
            if let Some((owner, name, portal2)) = self
                .h
                .x
                .portal_owner(unit)
                .or_else(|| self.portal_owner_from_links(game, unit))
            {
                // u32@1 the owner, the name, u32@21 this portal's GUID,
                // u32@25 its partner's (-1: none). A pair of portals sends
                // each other's GUIDs: the field portal's 0x82 carries
                // (0x22, 0x23), the town portal's (0x23, 0x22).
                let m = messages::portal_ownership(owner, &name, guid, portal2);
                self.h.x.send(receiver, &m);
            }
        }
    }

    /// S->C 0x82's fields: the portal's owner GUID o; the player with GUID
    /// o (none, o = -1 or gone: no 0x82); its name (the join name); the
    /// partner's GUID ([`View::portal_partner`]; -1 without one).
    fn portal_owner_from_links(&self, game: &Game, portal: UnitId) -> Option<(u32, Vec<u8>, u32)> {
        let owner = self.h.objects.as_ref()?.control.data.get(&portal)?.owner?;
        let owner = owner as u32;
        let player = game
            .lists
            .units_of_type(UnitType::Player)
            .into_iter()
            .find(|p| game.lists.unit(*p).is_some_and(|e| e.guid == owner))?;
        let raw = self
            .h
            .session
            .names
            .get(&player)
            .copied()
            .unwrap_or_default();
        let len = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
        let pair = self
            .portal_partner(game, portal)
            .and_then(|p| game.lists.unit(p))
            .map_or(0xFFFF_FFFF, |e| e.guid);
        Some((owner, raw[..len].to_vec(), pair))
    }

    /// Whether the client's room is ready; a client with no room is a
    /// wiring error ([`WiringError::NoClientRoom`]).
    pub fn client_room_ready(&mut self, game: &Game, client: ClientId) -> bool {
        let Some(room) = game.lists.client(client).and_then(|e| e.room) else {
            self.h.errors.push(WiringError::NoClientRoom(client));
            return false;
        };
        self.h.drlg.room_ready(game, room).unwrap_or(false)
    }
}

impl<X: Pending> View<'_, X> {
    /// Refresh room streaming for a new live view gap without triggering a
    /// player room switch or sending a room load.
    pub fn resize_view_gap(&mut self, game: &mut Game, gap: i32) -> Result<(), WiringError> {
        let changes = self.h.drlg.resize_view_gap(&mut game.lists, gap)?;
        for (client, switch) in changes {
            if let Some(player) = game.lists.client(client).and_then(|e| e.player) {
                self.switch_messages(game, client, player, None, &switch);
            }
            if let Some(entry) = game.lists.client_mut(client) {
                entry.view_rooms = switch.view;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod corpse_tests {
    /// The part-B corpse 0x74: 10 bytes, flag 1, the owner's GUID then the
    /// corpse's.
    #[test]
    fn corpse_assign_bytes() {
        assert_eq!(
            super::corpse_assign(0x0102_0304, 0x0506_0708),
            [0x74, 1, 4, 3, 2, 1, 8, 7, 6, 5]
        );
    }
}
