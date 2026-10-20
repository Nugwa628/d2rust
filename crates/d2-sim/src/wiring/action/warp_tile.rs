//! Warp tile units and the C→S 0x13 tile case.
//!
//! A level warp is a tile unit (unit type 5, class = the lvlwarp `Id`):
//! the DRLG's warp tile preset adds it to the room's preset units; the
//! population preset pass hands the list out once per DRLG room
//! ([`View::take_presets`], flag 0x4000000) and its first walk creates the
//! tiles before any preset monster ([`View::spawn_preset_units`]); a
//! deactivated room's tiles are stored and freed, and its restore
//! re-creates them as new units ([`View::create_tile`]);
//! [`View::warp_tile_message`] runs the walk into the warp for a C→S 0x13
//! whose unit type is 5.
//!
//! The first walk creates the tiles with the room's other non-monster
//! presets, in list order ([`View::spawn_preset_units`]).
//! The 0x13 tile case finds the tile by GUID and measures the unit
//! distance from the player: under 5 it warps, 5 to 50 it walks to the
//! tile and answers 0, else it answers 1.

use crate::drlg::room_flags;
use crate::game::Game;
use crate::units::lifecycle::AllocRequest;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::path::{place, PathCtx};

use super::{Pending, View};

/// The unit type of a host-placed monster preset: a level types provider's
/// own preset-list entry (class = monstats class) that population does not
/// read (it takes type 1). D2RUST's own type, used only by the play
/// preview's synthetic levels. Real levels never need it: their DS1
/// presets use type 1 (monster), 2 (object) and 4 (item), and the preset
/// walk creates Duriel or the Burial Grounds' Blood Raven (class 267,
/// mode 1, quest chain 2) from those (check `w3-01-burial-blood-raven`).
pub const HOST_MONSTER_PRESET: u32 = 6;
/// Preset unit type for an object the host's level types provider lists
/// (the Duriel preview level; D2RUST's own, see above): created through the
/// object state's `create_object` (allocation and the object init).
pub const HOST_OBJECT_PRESET: u32 = 7;

/// The unit type of an object preset (DS1 object entries).
pub const OBJECT_PRESET: u32 = 2;

/// The object classes whose preset path the quests keep as a map AI
/// (459 Anya's dummy, 461 Nihlathak's, 543 Larzuk's).
const MAP_AI_STORE_CLASSES: [u32; 3] = [459, 461, 543];

/// The unit type of a tile preset.
const TILE_PRESET: u32 = crate::path::warp::TILE_UNIT_TYPE as u32;

/// Unit flags the preset creator sets on every unit it creates (tiles
/// included).
pub const CREATED_UNIT_FLAGS: u32 = 0x300_0000;

impl<X: Pending> View<'_, X> {
    /// Whether the active `room`'s DRLG room hands its preset list out. A
    /// client copy always does; else the first call sets flag 0x4000000 and does,
    /// every later call finds the flag and gets none (no code clears it).
    /// A room without a DRLG room has no list; `true` then (nothing to
    /// hand out either way).
    pub fn take_presets(&mut self, game: &mut Game, room: RoomId) -> bool {
        let Some(act) = game.lists.room(room).map(|r| r.act) else {
            return true;
        };
        self.h
            .drlg
            .with_act(act, &mut game.lists, |d, _| {
                let Some(r) = d.drlg_room_of(room) else {
                    return true;
                };
                if d.on_client {
                    return true;
                }
                let flags = &mut d.room_mut(r).flags;
                if *flags & room_flags::PRESETS_HANDED_OUT != 0 {
                    return false;
                }
                *flags |= room_flags::PRESETS_HANDED_OUT;
                true
            })
            .unwrap_or(true)
    }

    /// One type-5 preset: a tile unit at (x, y) ([`Self::create_tile`],
    /// mode 0, flags 0: the warp preset adds no done bit) unless the room
    /// has one of that class there. True when allocated.
    fn spawn_tile_preset(
        &mut self,
        game: &mut Game,
        room: RoomId,
        (x, y): (i32, i32),
        class: u32,
    ) -> bool {
        if self.room_tile_at(game, room, class, (x, y)).is_some() {
            return false;
        }
        self.create_tile(game, room, class, x, y, 0, 0).is_some()
    }

    /// The preset creator sets unit flags 0x3000000 on every unit it
    /// creates: flag 0x2000000 makes the unit `S` of the compress, so a
    /// preset object of a level without `SaveMonsters` (the towns) is
    /// stored when its room is freed and restored by the room's next
    /// population. True when `made`.
    fn mark_preset_unit(&mut self, made: Option<UnitId>) -> bool {
        let Some(u) = made else {
            return false;
        };
        if let Some(r) = self.units.get_mut(u) {
            r.flags |= CREATED_UNIT_FLAGS;
        }
        true
    }

    /// Stores the quest map AI of a preset object with a path, on the lent
    /// quest host.
    fn map_ai_store(&mut self, game: &mut Game, class: u16, path: Vec<(u32, i32, i32)>) {
        let Some(mut host) = self.h.quest_host.take() else {
            return;
        };
        host.map_ai_store(game, self, class, path);
        self.h.quest_host = Some(host);
    }

    /// One type-2 preset: an object at (x, y) through the object state's
    /// `create_object`, unless the room has one of that class there. True
    /// when created.
    fn spawn_object_preset(
        &mut self,
        game: &mut Game,
        room: RoomId,
        (x, y): (i32, i32),
        class: u32,
    ) -> bool {
        let exists = game.lists.room_units(room).into_iter().any(|u| {
            game.lists.unit(u).is_some_and(|e| e.ty == UnitType::Object)
                && self.units.get(u).is_some_and(|r| r.class == class)
                && self.h.path_position(u) == (x, y)
        });
        if exists {
            return false;
        }
        let made = self.create_object(game, room, class, x, y, 0);
        self.mark_preset_unit(made)
    }

    /// The first preset walk: every non-monster preset of the active
    /// `room`'s DRLG room in list order (head first; warp tiles are
    /// prepended, so the last-added warp comes first): type 2 an object
    /// (`Self::spawn_object_preset`), type 5 a tile
    /// (`Self::spawn_tile_preset`). Before the monster walk; the
    /// caller has taken the list ([`Self::take_presets`]). Returns the
    /// number created.
    ///
    /// A preset whose unit (same class and position) is already in the
    /// room is skipped (the list is handed out once per room, so this
    /// only guards a double creation).
    pub fn spawn_preset_units(&mut self, game: &mut Game, room: RoomId) -> usize {
        let Some(act) = game.lists.room(room).map(|r| r.act) else {
            return 0;
        };
        let found = self.h.drlg.with_act(act, &mut game.lists, |d, svc| {
            let r = d.drlg_room_of(room)?;
            let origin = d.active_room(r)?.subtiles;
            let init_seed = d.init_seed;
            let units = svc.types.preset_units(d, r);
            let paths: Vec<_> = units
                .iter()
                .enumerate()
                .map(|(i, u)| {
                    if u.unit_type == OBJECT_PRESET && MAP_AI_STORE_CLASSES.contains(&u.class) {
                        svc.types.unit_path(d, r, i)
                    } else {
                        None
                    }
                })
                .collect();
            Some((units, paths, origin, init_seed))
        });
        let Some(Some((units, paths, origin, init_seed))) = found else {
            return 0;
        };
        let level = self.h.drlg.level_id(game, room).unwrap_or(0);
        let mut n = 0;
        for (p, path) in units.iter().zip(paths) {
            if p.unit_type == OBJECT_PRESET
                && preset_object_skipped(game, level, p.class, init_seed)
            {
                continue;
            }
            let at = (origin.x + p.x, origin.y + p.y);
            let made = match p.unit_type {
                OBJECT_PRESET => {
                    let made = self.spawn_object_preset(game, room, at, p.class);
                    if let (true, Some(path)) = (made, path) {
                        self.map_ai_store(game, p.class as u16, path);
                    }
                    made
                }
                TILE_PRESET => self.spawn_tile_preset(game, room, at, p.class),
                _ => false,
            };
            n += usize::from(made);
        }
        n
    }

    /// Creates a new tile unit (new GUID) through the preset creator, then
    /// unit flags |= 0x3000000 over `flags`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_tile(
        &mut self,
        game: &mut Game,
        room: RoomId,
        class: u32,
        x: i32,
        y: i32,
        mode: u32,
        flags: u32,
    ) -> Option<UnitId> {
        let req = AllocRequest {
            ty: UnitType::Tile,
            class,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode,
            allied: false,
        };
        let u = self.allocate(game, &req, x, y)?;
        if let Some(r) = self.units.get_mut(u) {
            r.flags |= flags | CREATED_UNIT_FLAGS;
        }
        Some(u)
    }

    /// A tile unit of `class` at `at` in the active `room`'s unit list.
    fn room_tile_at(
        &self,
        game: &Game,
        room: RoomId,
        class: u32,
        at: (i32, i32),
    ) -> Option<UnitId> {
        game.lists.room_units(room).into_iter().find(|&u| {
            game.lists.unit(u).is_some_and(|e| e.ty == UnitType::Tile)
                && self.units.get(u).is_some_and(|r| r.class == class)
                && self.h.path_position(u) == at
        })
    }

    /// Allocates a monster for each [`HOST_MONSTER_PRESET`] entry of the
    /// active `room`'s DRLG room that has none yet (same class), as the
    /// tiles are, with its boss chains linked. Returns the created (unit,
    /// class) pairs.
    pub fn spawn_host_monsters(&mut self, game: &mut Game, room: RoomId) -> Vec<(UnitId, u32)> {
        let Some(act) = game.lists.room(room).map(|r| r.act) else {
            return Vec::new();
        };
        let found = self.h.drlg.with_act(act, &mut game.lists, |d, svc| {
            let r = d.drlg_room_of(room)?;
            let origin = d.active_room(r)?.subtiles;
            Some((svc.types.preset_units(d, r), origin))
        });
        let Some(Some((units, origin))) = found else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for p in units.iter().filter(|p| p.unit_type == HOST_MONSTER_PRESET) {
            let exists = game.lists.room_units(room).into_iter().any(|u| {
                game.lists
                    .unit(u)
                    .is_some_and(|e| e.ty == UnitType::Monster)
                    && self.units.get(u).is_some_and(|r| r.class == p.class)
            });
            if exists {
                continue;
            }
            let req = AllocRequest {
                ty: UnitType::Monster,
                class: p.class,
                room: Some(room),
                add: true,
                fixed_guid: None,
                mode: 1,
                allied: false,
            };
            if let Some(u) = self.allocate(game, &req, origin.x + p.x, origin.y + p.y) {
                if let Some(r) = self.units.get_mut(u) {
                    r.flags |= crate::missiles::unit_flag::IS_VALID_TARGET
                        | crate::missiles::unit_flag::CAN_BE_ATTACKED;
                }
                self.link_boss_chains(u, p.class);
                out.push((u, p.class));
            }
        }
        out
    }

    /// Links the quest chains that the creation-time boss mods link, for a
    /// monster placed without the creation function; a chain already
    /// linked is refused.
    pub fn link_boss_chains(&mut self, u: UnitId, class: u32) {
        let chains: Vec<u32> =
            crate::monsters::init::boss_chains(&self.h.tables.combat.monstats, class).collect();
        for n in chains {
            // No game here: applied at the next quest call.
            self.queue_quest_link(u, n);
        }
    }

    /// Creates an object for each [`HOST_OBJECT_PRESET`] entry of the
    /// active `room`'s DRLG room that has none yet (same class).
    pub fn spawn_host_objects(&mut self, game: &mut Game, room: RoomId) {
        let Some(act) = game.lists.room(room).map(|r| r.act) else {
            return;
        };
        let found = self.h.drlg.with_act(act, &mut game.lists, |d, svc| {
            let r = d.drlg_room_of(room)?;
            let origin = d.active_room(r)?.subtiles;
            Some((svc.types.preset_units(d, r), origin))
        });
        let Some(Some((units, origin))) = found else {
            return;
        };
        for p in units.iter().filter(|p| p.unit_type == HOST_OBJECT_PRESET) {
            let exists = game.lists.room_units(room).into_iter().any(|u| {
                game.lists.unit(u).is_some_and(|e| e.ty == UnitType::Object)
                    && self.units.get(u).is_some_and(|r| r.class == p.class)
            });
            if !exists {
                self.create_object(game, room, p.class, origin.x + p.x, origin.y + p.y, 0);
            }
        }
    }

    /// The first tile unit of `class` in the active `room`'s unit list.
    pub fn room_tile(&self, game: &Game, room: RoomId, class: u32) -> Option<UnitId> {
        game.lists.room_units(room).into_iter().find(|&u| {
            game.lists.unit(u).is_some_and(|e| e.ty == UnitType::Tile)
                && self.units.get(u).is_some_and(|r| r.class == class)
        })
    }

    /// The C→S 0x13 tile case: the tile with `guid`, at unit distance < 5
    /// the warp of `player`,
    /// at 5..=50 the walk to the tile. `None`: no path provider.
    /// 0: the warp ran ([`WarpOutcome::Arrived`]) or the walk started;
    /// 1: nothing happened (no tile, too far, or the warp did nothing).
    ///
    /// [`WarpOutcome::Arrived`]: crate::path::warp::WarpOutcome::Arrived
    pub fn warp_tile_message(&mut self, game: &mut Game, player: UnitId, guid: u32) -> Option<u32> {
        self.h.paths.as_ref()?;
        let Some(tile) = game.lists.find_unit(UnitType::Tile, guid) else {
            return Some(1);
        };
        let room = game.lists.unit(tile).and_then(|e| e.room());
        let class = self.units.get(tile).map(|r| r.class);
        let (Some(room), Some(class)) = (room, class) else {
            return Some(1);
        };
        let dist = self.object_unit_distance(player, tile);
        if dist >= 5 {
            if dist > 50 {
                return Some(1);
            }
            let started = PathCtx::of(self, game).approach_unit(player, tile);
            if started.is_some() {
                self.h.tile_walks.push((player, guid));
            }
            return Some(u32::from(started.is_none()));
        }
        let c = PathCtx::of(self, game);
        let r = place::warp_player(c, player, room, class);
        Some(u32::from(
            r != Some(crate::path::warp::WarpOutcome::Arrived),
        ))
    }
}

/// The first walk's per-level object skips: levels 133 and 135 never place
/// object 397, level 134 never places the waypoint 402 and drops the
/// object preset that brings the game's count ([`Game::sands_preset_objects`])
/// to `init seed % 3 + 3`, level 136 never places objects 26, 268 and 269.
fn preset_object_skipped(game: &mut Game, level: u32, class: u32, init_seed: u32) -> bool {
    match level {
        133 | 135 => class == 397,
        134 => {
            if class == 402 {
                return true;
            }
            game.sands_preset_objects = game.sands_preset_objects.wrapping_add(1);
            game.sands_preset_objects == init_seed % 3 + 3
        }
        136 => matches!(class, 26 | 268 | 269),
        _ => false,
    }
}

#[cfg(test)]
mod preset_skip_tests {
    use super::*;

    #[test]
    fn levels_133_135_136_skip_their_objects() {
        let mut g = Game::default();
        assert!(preset_object_skipped(&mut g, 133, 397, 0));
        assert!(preset_object_skipped(&mut g, 135, 397, 0));
        assert!(!preset_object_skipped(&mut g, 133, 398, 0));
        for c in [26, 268, 269] {
            assert!(preset_object_skipped(&mut g, 136, c, 0));
        }
        assert!(!preset_object_skipped(&mut g, 136, 397, 0));
        assert!(!preset_object_skipped(&mut g, 37, 397, 0));
        assert_eq!(g.sands_preset_objects, 0);
    }

    #[test]
    fn level_134_skips_the_waypoint_and_the_counted_object() {
        // init seed 7: 7 % 3 + 3 = 4, so the fourth counted object goes.
        let mut g = Game::default();
        assert!(preset_object_skipped(&mut g, 134, 402, 7));
        let skipped: Vec<bool> = (0..6)
            .map(|i| preset_object_skipped(&mut g, 134, 100 + i, 7))
            .collect();
        assert_eq!(skipped, [false, false, false, true, false, false]);
        assert_eq!(g.sands_preset_objects, 6);
    }
}
