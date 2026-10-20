//! Portal pairs and the Town Portal cast.
//!
//! [`View::create_portal_pair`] makes a portal pair for every caller (the
//! cast, the quest portals, Tyrael's): object 1 in mode 1 at the caller's
//! spot, object 2 in mode 2 at the destination level's tile-11 spawn
//! point, linked to each other ([`PortalLinks`]). [`View::town_portal_cast`]
//! is the Town Portal item use: the town refusal, the old pair closed, the
//! new one made, its owners and the player's portal GUID set.
//!
//! A portal leaves the game through its room's delete list
//! ([`View::remove_portal_object`]): the per-client update sends S→C 0x0A
//! to every client whose room adjacency holds that room
//! ([`View::send_room_deletes`]).

use std::collections::BTreeMap;

use crate::drlg::{act_of_level, is_town, TOWN_LEVELS};
use crate::game::Game;
use crate::path::coords::Point;
use crate::path::place_seams::{mask, CollisionView};
use crate::units::{ClientId, RoomId, UnitId, UnitType};
use crate::wiring::path::place::{level_spawn, Rooms};

use super::{Pending, View, WiringError};

/// The class of the town portal object.
pub const TOWN_PORTAL_CLASS: u32 = 59;
/// The class of the permanent portal.
const PERMANENT_PORTAL_CLASS: u32 = 60;
/// Free-point size of both ends.
const PORTAL_SIZE: i32 = 3;
/// Spawn tile index of object 2's destination.
const DEST_TILE: u32 = 11;
/// Step of the destination free point.
const DEST_STEP: i32 = 5;
/// Tyrael's arrival hook: level 40's tile-12 spawn, free point step 7.
const TYRAEL_SOURCE: u32 = 73;
const TYRAEL_TOWN: u32 = 40;
const TYRAEL_TILE: u32 = 12;
const TYRAEL_STEP: i32 = 7;
/// Exception to the town refusal: class 60 to these levels opens from a town.
const TOWN_OPEN_LEVELS: [u32; 5] = [39, 133, 134, 135, 136];
/// Pandemonium Finale refuses a Town Portal cast like a town.
const FINALE: u32 = 136;
/// Sound events: `notintown`, `player_townportal_cast`.
const SOUND_NOT_IN_TOWN: u16 = 24;
const SOUND_CAST: u16 = 7;
/// Unit flag-ex bit of a linked portal.
const FLAG_EX_PARTNER: u32 = 0x400;
/// State 98 and stats 353 / 354 of a linked unit with a stat list.
const STATE_PARTNER: u16 = 98;
const STAT_PARTNER_TYPE: u16 = 353;
const STAT_PARTNER_GUID: u16 = 354;
/// Unit flags bit the cast sets.
const UNIT_FLAG_CAST: u32 = 0x40;
/// Fatal assert codes of the portal pair creation (no room, cross-act
/// destination, source level above 255).
const FATAL_NO_ROOM: u32 = 0xE42;
const FATAL_CROSS_ACT: u32 = 0xE5C;
const FATAL_SOURCE_LEVEL: u32 = 0xE2B;

/// One end's link to the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalLink {
    /// The other's unit type.
    pub ty: u8,
    /// The other's GUID.
    pub guid: u32,
    /// The other's x position.
    pub x: i32,
    /// The other's y position.
    pub y: i32,
    /// The tile x / y of the other's room
    /// (none: the other had no room, the fields are not written).
    pub tile: Option<(i32, i32)>,
}

/// The portal links and the players' portal GUIDs.
#[derive(Debug, Clone, Default)]
pub struct PortalLinks {
    /// Per linked portal unit.
    links: BTreeMap<UnitId, PortalLink>,
    /// The GUID of the
    /// player's town portal (object 1 of its last pair). Never cleared.
    player_portal: BTreeMap<UnitId, u32>,
    /// Portals an object call removed, freed when it returns.
    doomed: Vec<UnitId>,
}

impl PortalLinks {
    /// `portal`'s link to its partner.
    pub fn link(&self, portal: UnitId) -> Option<&PortalLink> {
        self.links.get(&portal)
    }

    /// The GUID of `player`'s town portal (`None`: never written).
    pub fn player_portal(&self, player: UnitId) -> Option<u32> {
        self.player_portal.get(&player).copied()
    }

    /// Queues `object` for removal once the object call returns
    /// ([`View::flush_portal_removals`]).
    pub(super) fn doom(&mut self, object: UnitId) {
        self.doomed.push(object);
    }
}

impl<X: Pending> View<'_, X> {
    /// The partner of `portal`: the object unit its link names.
    /// `None`: no link, or the partner is gone. The pair's rooms carry the
    /// "has portal" flag, so the partner's room is never freed under it; it
    /// is not streamed here.
    pub fn portal_partner(&self, game: &Game, portal: UnitId) -> Option<UnitId> {
        let l = self.h.portals.link(portal)?;
        let ty = *UnitType::ALL.get(usize::from(l.ty))?;
        if ty != UnitType::Object {
            return None;
        }
        game.lists.find_unit(ty, l.guid)
    }

    /// Creates a portal pair: the result (1: both objects
    /// exist) and object 1. `tyrael`: Tyrael's portal (quest chain 13 is
    /// active during the call), so the arrival hook places object 2.
    /// Neither end's owner GUID is set.
    #[allow(clippy::too_many_arguments)]
    pub fn create_portal_pair(
        &mut self,
        game: &mut Game,
        owner: Option<UnitId>,
        room: Option<RoomId>,
        (x, y): (i32, i32),
        level: u32,
        class: u32,
        exact: bool,
        tyrael: bool,
    ) -> (u32, Option<UnitId>) {
        // A room is required.
        let Some(room) = room else {
            self.h.errors.push(WiringError::Portal(FATAL_NO_ROOM));
            return (0, None);
        };
        let Some(room_level) = self.h.drlg.level_id(game, room) else {
            return (0, None);
        };
        // Town refusal.
        if let Some(o) = owner {
            let open = TOWN_OPEN_LEVELS.contains(&level) && class == PERMANENT_PORTAL_CLASS;
            if is_town(room_level) && !open {
                self.portal_sound(game, o, SOUND_NOT_IN_TOWN, Some(o));
                return (0, None);
            }
        }
        // The destination must be in the same act.
        if act_of_level(level) != act_of_level(room_level) {
            self.h.errors.push(WiringError::Portal(FATAL_CROSS_ACT));
            return (0, None);
        }
        // Find a free spot unless `exact`.
        let mut p = Point::new(x, y);
        if !exact && !self.portal_spot(room, &mut p) {
            return (0, None);
        }
        let Some(r1) = Rooms(&self.h.drlg).cell_room(Some(room), p.x, p.y) else {
            return (0, None);
        };
        // Object 1 (its init sets its destination), then
        // the mode set (already mode 1: the update mark only).
        let Some(o1) = self.create_object(game, r1, class, p.x, p.y, 1) else {
            return (0, None);
        };
        self.object_set_mode(game, o1, 1);
        // No far end: object 1 is already freed.
        let Some(o2) = self.portal_far_end(game, o1, level, room_level, tyrael) else {
            return (0, None);
        };
        // Mark both ends.
        self.or_portal_flags(o2, 3);
        self.h.drlg.refresh_room(game, r1, false);
        if let Some(r2) = game.lists.unit(o2).and_then(|e| e.room()) {
            self.h.drlg.refresh_room(game, r2, false);
        }
        (1, Some(o1))
    }

    /// Populates `room` inside the call
    /// on the lent monster world; none lent: left to the room pass.
    pub(crate) fn populate_off_tick(&mut self, game: &mut Game, room: RoomId) {
        let View {
            units,
            stats,
            data,
            h,
            ..
        } = self;
        let mut sim = crate::units::hooks::Sim {
            game,
            units,
            stats,
            data,
        };
        h.with_monster_world(|w, h| w.populate_off_tick(&mut sim, h, room));
    }

    /// The free spot for object 1: the field search from `p` (size 3,
    /// item-floor / portal-field masks, no fallback); `false`: none.
    ///
    /// The game always has the field; the play host loads it into
    /// `PathState::field` (`d2-client` `app/single_player.rs`). Test-only:
    /// a fixture without the field runs the same search without the field
    /// test.
    fn portal_spot(&self, room: RoomId, p: &mut Point) -> bool {
        let rooms = Rooms(&self.h.drlg);
        let field = self.h.paths.as_ref().and_then(|s| s.field.clone());
        let origin = *p;
        let r = match field.as_deref() {
            Some(f) => crate::path::search::free_point_field(
                &rooms,
                f,
                Some(room),
                p,
                origin,
                PORTAL_SIZE,
                mask::ITEM_FLOOR,
                mask::PORTAL_FIELD,
                false,
            ),
            None => crate::path::search::free_point(
                &rooms,
                Some(room),
                p,
                PORTAL_SIZE,
                mask::ITEM_FLOOR,
                false,
            ),
        };
        matches!(r, Ok(Some(_)))
    }

    /// Makes object 2 and links it to object 1; `None` (object 1 removed).
    ///
    /// The destination spawn room and Tyrael's town room are populated
    /// inside the call, so the far end's units come before object 2 (check
    /// `w1-w6-cow-portal`, state frame 8): on the lent monster world
    /// (`MonsterWorld::populate_off_tick`); with none lent (action-only
    /// hosts) the room pass populates them later.
    fn portal_far_end(
        &mut self,
        game: &mut Game,
        o1: UnitId,
        level: u32,
        source_level: u32,
        tyrael: bool,
    ) -> Option<UnitId> {
        let act = act_of_level(level);
        // The destination spawn point.
        let spawn = level_spawn(self, game, act, level, DEST_TILE);
        if let Some((s, _, _)) = spawn {
            self.populate_off_tick(game, s);
        }
        let Some((s, sx, sy)) = spawn else {
            self.remove_portal_object(game, o1);
            return None;
        };
        // The arrival hook (only source level 73 with Tyrael's flag
        // moves the far end to Tyrael's town spawn).
        let mut p = Point::new(sx, sy);
        let mut r = s;
        let mut hooked = false;
        if source_level == TYRAEL_SOURCE && tyrael {
            let town_act = act_of_level(TYRAEL_TOWN);
            if let Some((t, tx, ty)) = level_spawn(self, game, town_act, TYRAEL_TOWN, TYRAEL_TILE) {
                self.populate_off_tick(game, t);
                let mut q = p;
                let found = crate::path::search::free_point_step(
                    &Rooms(&self.h.drlg),
                    Some(t),
                    &mut q,
                    PORTAL_SIZE,
                    mask::PORTAL_DEST,
                    TYRAEL_STEP,
                );
                if let Ok(Some(f)) = found {
                    p = Point::new(tx, ty);
                    r = f;
                    hooked = true;
                }
            }
        }
        // Resolve the room of the chosen point.
        let r = if hooked {
            Rooms(&self.h.drlg).cell_room(Some(r), p.x, p.y)
        } else {
            Some(s)
        };
        let r = r.unwrap_or_else(|| {
            p = Point::new(sx, sy);
            s
        });
        // Free point near the spawn: a failed search leaves p as given.
        let found = crate::path::search::free_point_step(
            &Rooms(&self.h.drlg),
            Some(r),
            &mut p,
            PORTAL_SIZE,
            mask::PORTAL_DEST,
            DEST_STEP,
        );
        let r = match found {
            Ok(Some(f)) => f,
            _ => level_spawn(self, game, act, level, DEST_TILE).map_or(s, |(room, _, _)| room),
        };
        // Create object 2.
        let class = self.units.get(o1).map_or(TOWN_PORTAL_CLASS, |u| u.class);
        let Some(o2) = self.create_object(game, r, class, p.x, p.y, 2) else {
            self.remove_portal_object(game, o1);
            return None;
        };
        let src = game
            .lists
            .unit(o1)
            .and_then(|e| e.room())
            .and_then(|room| self.h.drlg.level_id(game, room))
            .unwrap_or(0);
        let Ok(src) = u8::try_from(src) else {
            self.h.errors.push(WiringError::Portal(FATAL_SOURCE_LEVEL));
            return None;
        };
        if let Some(d) = self.object_data_mut(o2) {
            d.interact = src;
            d.portal_flags |= 3;
        }
        self.object_set_mode(game, o2, 2);
        // Link the pair.
        self.link_portals(game, o1, o2);
        Some(o2)
    }

    /// Links two portals: each end's link record (position and room tile of
    /// the other), then the partner both ways.
    fn link_portals(&mut self, game: &Game, a: UnitId, b: UnitId) {
        for (u, other) in [(a, b), (b, a)] {
            let Some(e) = game.lists.unit(other) else {
                continue;
            };
            let (ty, guid) = (e.ty as u8, e.guid);
            let (x, y) = self.h.path_position(other);
            let tile = e
                .room()
                .and_then(|r| self.h.drlg.drlg_room(game, r))
                .map(|(d, dr)| (d.room(dr).rect.x, d.room(dr).rect.y));
            self.h.portals.links.insert(
                u,
                PortalLink {
                    ty,
                    guid,
                    x,
                    y,
                    tile,
                },
            );
            if let Some(r) = self.units.get_mut(u) {
                r.flags2 |= FLAG_EX_PARTNER;
            }
            if self.stats.unit_list(u).is_some()
                && usize::from(STATE_PARTNER) < self.stats.data().states.count()
            {
                self.set_state(u, STATE_PARTNER, true);
                self.set_base(u, STAT_PARTNER_TYPE, i32::from(ty));
                self.set_base(u, STAT_PARTNER_GUID, guid as i32);
            }
        }
    }

    /// The Town Portal cast: the result (1 made, 0 not) and whether the
    /// cast's own S→C 0x7C for the item is due (not after a non-player
    /// caster or a town refusal); the caller holds the item and sends it.
    pub fn town_portal_cast(&mut self, game: &mut Game, player: UnitId) -> (u32, bool) {
        // Only a player casts.
        if self.units.get(player).map(|r| r.ty) != Some(UnitType::Player) {
            return (0, false);
        }
        // Mark the cast on the unit.
        if let Some(r) = self.units.get_mut(player) {
            r.flags |= UNIT_FLAG_CAST;
        }
        // The caster's room and level.
        let room = game.lists.unit(player).and_then(|e| e.room());
        // No room: the town test of a null room is 0 and its level is not
        // 136, so the old pair is closed, then the pair creation fails with
        // the fatal no-room code 0xE42 (the process ends in the game).
        let Some(room) = room else {
            self.close_player_portal(game, player);
            self.h.errors.push(WiringError::Portal(FATAL_NO_ROOM));
            return (0, false);
        };
        let room = Some(room);
        let Some(level) = room.and_then(|r| self.h.drlg.level_id(game, r)) else {
            return (0, false);
        };
        let town = TOWN_LEVELS[usize::from(act_of_level(level))];
        // Towns and the Finale refuse.
        if is_town(level) || level == FINALE {
            self.portal_sound(game, player, SOUND_NOT_IN_TOWN, Some(player));
            return (0, false);
        }
        // Close the old pair.
        self.close_player_portal(game, player);
        // Make the new pair at the caster's position.
        let (x, y) = self.h.path_position(player);
        let (made, o1) = self.create_portal_pair(
            game,
            Some(player),
            room,
            (x, y),
            town,
            TOWN_PORTAL_CLASS,
            false,
            false,
        );
        // Cast sound.
        self.portal_sound(game, player, SOUND_CAST, None);
        // Record the owner and open both portals.
        if made != 0 {
            if let Some(o1) = o1 {
                let pg = game.lists.unit(player).map_or(0, |e| e.guid);
                let g1 = game.lists.unit(o1).map_or(u32::MAX, |e| e.guid);
                self.h.portals.player_portal.insert(player, g1);
                if let Some(d) = self.object_data_mut(o1) {
                    d.owner = Some(pg as i32);
                }
                if let Some(o2) = self.portal_partner(game, o1) {
                    if let Some(d) = self.object_data_mut(o2) {
                        d.owner = Some(pg as i32);
                    }
                    self.h.x.object_portal_opened(o2);
                }
                self.h.x.object_portal_opened(o1);
            }
        }
        // The cast's own message is due.
        (made, true)
    }

    /// Closes the player's town portal: the class-59
    /// object whose GUID is the player's portal GUID and its partner leave
    /// the game, each after the Act V hook
    /// ([`Pending::object_portal_act5`]). The GUID stays.
    pub fn close_player_portal(&mut self, game: &mut Game, player: UnitId) {
        let Some(g) = self.h.portals.player_portal(player) else {
            return;
        };
        let Some(o) = game.lists.find_unit(UnitType::Object, g) else {
            return;
        };
        if self.units.get(o).map(|r| r.class) != Some(TOWN_PORTAL_CLASS) {
            return;
        }
        self.h.x.object_portal_act5(o);
        let q = self.portal_partner(game, o);
        self.remove_portal_object(game, o);
        if let Some(q) = q {
            self.h.x.object_portal_act5(q);
            self.remove_portal_object(game, q);
        }
    }

    /// A portal leaves the game: its room's delete
    /// list gets {type, GUID}, the unit is freed
    /// and the room refreshed (the
    /// "has portal" flag cleared).
    pub fn remove_portal_object(&mut self, game: &mut Game, object: UnitId) {
        let Some((ty, guid, room)) = game
            .lists
            .unit(object)
            .map(|e| (e.ty as u8, e.guid, e.room()))
        else {
            return;
        };
        if let Some(r) = room {
            self.room_delete(game, r, ty, guid);
        }
        self.h.portals.links.remove(&object);
        self.remove(game, object);
        if let Some(r) = room {
            self.h.drlg.refresh_room(game, r, true);
        }
    }

    /// Frees the portals an object call removed
    /// (`ObjectHost::remove_portal`), in order.
    pub fn flush_portal_removals(&mut self, game: &mut Game) {
        for u in std::mem::take(&mut self.h.portals.doomed) {
            self.remove_portal_object(game, u);
        }
    }

    /// Prepends a record to the room's
    /// delete list and sets the act's removal flag.
    pub fn room_delete(&mut self, game: &mut Game, room: RoomId, ty: u8, guid: u32) {
        self.h
            .room_deletes
            .entry(room)
            .or_default()
            .insert(0, (ty, guid));
        if let Some(a) = game
            .lists
            .room(room)
            .map(|r| r.act)
            .and_then(|act| game.lists.act_mut(act))
        {
            a.pending_removals = true;
        }
    }

    /// Sends the room deletes: for each room
    /// of the client's room adjacency (array order), each delete record
    /// (list order) other than the client's own player: S→C 0x0A.
    pub fn send_room_deletes(&mut self, game: &Game, client: ClientId) {
        let Some(c) = game.lists.client(client) else {
            return;
        };
        let (Some(player), Some(room)) = (c.player, c.room) else {
            return;
        };
        let own = game.lists.unit(player).map(|e| (e.ty as u8, e.guid));
        let adjacent = game
            .lists
            .room(room)
            .map(|r| r.adjacent.clone())
            .unwrap_or_default();
        for r in adjacent {
            let Some(list) = self.h.room_deletes.get(&r).cloned() else {
                continue;
            };
            for (ty, guid) in list {
                if Some((ty, guid)) != own {
                    let m = crate::units::messages::remove_unit(ty, guid);
                    self.h.x.send(player, &m);
                }
            }
        }
    }

    /// The object data of `object` (`None`: no object state or data).
    pub(super) fn object_data_mut(
        &mut self,
        object: UnitId,
    ) -> Option<&mut crate::world::objects::ObjectData> {
        self.h.objects.as_mut()?.control.data.get_mut(&object)
    }

    /// ORs `bits` into the object data's portal flags.
    fn or_portal_flags(&mut self, object: UnitId, bits: u8) {
        if let Some(d) = self.object_data_mut(object) {
            d.portal_flags |= bits;
        }
    }

    /// A sound event on the unit.
    fn portal_sound(&mut self, game: &mut Game, unit: UnitId, event: u16, target: Option<UnitId>) {
        if let Err(e) = crate::units::sound::queue_sound(game, unit, event, target) {
            self.unit_error(crate::game::GameError::from(e).into());
        }
    }
}
