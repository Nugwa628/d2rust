//! Light records and the client's light list.
//!
//! A light record is one light in the world: an owner (the unit carrying
//! it, or none), a position in eighths of a sub-tile, a radius that walks
//! one sub-tile per drawn frame towards its target, an intensity and a
//! colour. The list holds them newest first; every drawn frame walks it
//! from head to tail, moves each record to its owner, steps its radius,
//! drops it once it has died away and adds its light to the map.
//!
//! The client world is read through [`LightWorld`]. States the light code
//! cannot continue from are [`LightError`]s, never panics.

use super::contribute;
use super::map::LightMap;

/// Owner type of a record without owner.
pub const OWNER_NONE: u32 = 6;
/// Owner GUID of a record without owner.
pub const GUID_NONE: u32 = u32::MAX;
/// Largest radius in sub-tiles: larger requests are clamped to it.
pub const MAX_RADIUS: i32 = 18;

/// Unit types, numbered as the protocol numbers them (a record's owner
/// type).
pub mod unit_type {
    pub const PLAYER: u32 = 0;
    pub const MONSTER: u32 = 1;
    pub const OBJECT: u32 = 2;
    pub const MISSILE: u32 = 3;
    pub const ITEM: u32 = 4;
    pub const TILE: u32 = 5;
}

/// A room of the client, as the caller identifies it.
pub type RoomId = u32;

/// States the light code cannot continue from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LightError {
    /// Removing a record that is not in the list.
    #[error("light record not in the list")]
    NotInList,
    /// Letting a cached (kind 2) record die: cached lights are removed
    /// outright, never faded out.
    #[error("die on a cached (kind 2) light record")]
    DieOnCached,
    /// A new room appeared while a cached record's owner is gone or not
    /// found (the game's fatal error 0x591).
    #[error("fatal 0x591: cached light record without a found owner")]
    Fatal0x591,
    /// A new room appeared while a cached record's owner is in no room;
    /// what the room search should start from is not defined.
    #[error("cached light record owner has no room (unspecified)")]
    OwnerWithoutRoom,
    /// A cached record's shadow grid has to be built but its owner or the
    /// owner's room is gone.
    #[error("cached light record built without owner or room")]
    CacheWithoutOwner,
}

/// The owner of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Owner {
    pub unit_type: u32,
    pub guid: u32,
    /// The owner is a client-only unit (unit flag bit 21): it is looked up
    /// among the client's own units rather than the server's.
    pub client_only: bool,
}

/// What the new-room rule reads from the client world: owner lookups and
/// the room search.
pub trait LightRooms {
    /// The owner's precise position (16.16 sub-tiles), `None` when the
    /// owner is not found (among the client-only units when
    /// `client_only`, else among the server's).
    fn owner_position(&self, owner: &Owner) -> Option<(i32, i32)>;
    /// The owner's sub-tile: from its static path for objects, items and
    /// tiles, from its dynamic path for players, monsters and missiles (0
    /// without a path). `None` when the owner is not found.
    fn owner_subtile(&self, owner: &Owner) -> Option<(i32, i32)>;
    /// The owner's room; `None` when it has none.
    fn owner_room(&self, owner: &Owner) -> Option<RoomId>;
    /// The room holding sub-tile `(x, y)`, searched from `room` and its
    /// neighbours.
    fn cell_room(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId>;
}

/// What the light code reads from the client world.
pub trait LightWorld: LightRooms {
    /// Whether the owner is the local player.
    fn is_local_player(&self, owner: &Owner) -> bool;
    /// Whether sub-tile `(x, y)` blocks light, searched from the owner's
    /// room (the collision test for light-blocking walls).
    fn owner_blocks(&self, owner: &Owner, x: i32, y: i32) -> bool;
}

/// How a record's light is shaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LightKind {
    /// 0: shadowed by walls at the highest light quality, plain otherwise.
    /// Units' lights.
    Shadowed,
    /// 1: never shadowed. Short-lived lights (missiles, overlays, casts).
    Plain,
    /// 2: always shadowed, from a shadow grid built once and cached on the
    /// record. Lights that do not move (objects).
    Cached,
}

/// A light's position from a unit's precise 16.16 coordinate, in eighths
/// of a sub-tile: `(P >> 13) + 4`. The extra half sub-tile makes the
/// light's cell the unit's position rounded to the nearest sub-tile.
pub fn unit_light_pos(precise: i32) -> i32 {
    (precise >> 13) + 4
}

/// One light record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightRecord {
    pub owner_type: u32,
    pub owner_guid: u32,
    /// The owner is a client-only unit ([`Owner::client_only`]).
    pub lookup_flag: bool,
    pub kind: LightKind,
    /// Position in eighths of a sub-tile.
    pub x: i32,
    pub y: i32,
    /// Current radius in eighths; it walks towards `target` by one
    /// sub-tile per drawn frame.
    pub radius: i32,
    /// Target radius in eighths.
    pub target: i32,
    /// The light is fading out: it no longer follows its owner, its
    /// target is 0, and it leaves the list once its radius reaches 0.
    pub dying: bool,
    pub i: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Kind 2 only: whether `cache` holds a shadow grid for the current
    /// radius and surroundings.
    pub cache_valid: bool,
    /// Kind 2 only: the `(2m + 1)²` shade values around the light, `m` its
    /// radius in sub-tiles.
    pub cache: Vec<i32>,
}

impl LightRecord {
    /// The owner, `None` for a record without owner.
    pub fn owner(&self) -> Option<Owner> {
        (self.owner_type != OWNER_NONE).then_some(Owner {
            unit_type: self.owner_type,
            guid: self.owner_guid,
            client_only: self.lookup_flag,
        })
    }

    fn free_cache(&mut self) {
        self.cache_valid = false;
        self.cache = Vec::new();
    }
}

/// A record handle, stable while the record is in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LightId(pub u32);

/// The client's light list, newest record first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightList {
    records: Vec<(LightId, LightRecord)>,
    next_id: u32,
    /// Coloured light: when off, every lit cell loses its colour and only
    /// intensity is kept. On by default.
    pub colored: bool,
}

impl Default for LightList {
    fn default() -> Self {
        LightList {
            records: Vec::new(),
            next_id: 0,
            colored: true,
        }
    }
}

impl LightList {
    pub fn new() -> Self {
        Self::default()
    }

    /// The records, head (newest) first.
    pub fn iter(&self) -> impl Iterator<Item = (LightId, &LightRecord)> {
        self.records.iter().map(|(id, r)| (*id, r))
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn get(&self, id: LightId) -> Option<&LightRecord> {
        self.records.iter().find(|(i, _)| *i == id).map(|(_, r)| r)
    }

    pub(crate) fn get_mut(&mut self, id: LightId) -> Option<&mut LightRecord> {
        self.records
            .iter_mut()
            .find(|(i, _)| *i == id)
            .map(|(_, r)| r)
    }

    /// Creates a record at the head of the list. A radius `r` below 1
    /// creates nothing; above [`MAX_RADIUS`] it is clamped. `pos` is the
    /// light position in eighths of a sub-tile (for a unit:
    /// [`unit_light_pos`] of its precise position). A cached record gets an
    /// empty `(2m + 1)²` grid that its first contribution fills.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        owner: Option<Owner>,
        pos: (i32, i32),
        kind: LightKind,
        r: i32,
        i: u8,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Option<LightId> {
        if r < 1 {
            return None;
        }
        let r8 = 8 * r.min(MAX_RADIUS);
        let (owner_type, owner_guid, lookup_flag) = match owner {
            Some(o) => (o.unit_type, o.guid, o.client_only),
            None => (OWNER_NONE, GUID_NONE, false),
        };
        let cache = if kind == LightKind::Cached {
            let side = (2 * (r8 >> 3) + 1) as usize;
            vec![0; side * side]
        } else {
            Vec::new()
        };
        let rec = LightRecord {
            owner_type,
            owner_guid,
            lookup_flag,
            kind,
            x: pos.0,
            y: pos.1,
            radius: r8,
            target: r8,
            dying: false,
            i,
            r: red,
            g: green,
            b: blue,
            cache_valid: false,
            cache,
        };
        let id = LightId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.records.insert(0, (id, rec));
        Some(id)
    }

    /// Sets the radius at once (no walk): `r` ≤ 0 does nothing, larger
    /// than [`MAX_RADIUS`] is clamped, radius and target both become `r`.
    /// A cached record's grid no longer fits and is freed.
    pub fn set_radius(&mut self, id: LightId, r: i32) {
        if r <= 0 {
            return;
        }
        if let Some(rec) = self.get_mut(id) {
            let r8 = 8 * r.min(MAX_RADIUS);
            rec.radius = r8;
            rec.target = r8;
            if rec.kind == LightKind::Cached {
                rec.free_cache();
            }
        }
    }

    /// Sets the target radius the record walks to, one sub-tile per drawn
    /// frame: `r` ≤ 0 does nothing, larger than [`MAX_RADIUS`] is clamped.
    pub fn set_target(&mut self, id: LightId, r: i32) {
        if r <= 0 {
            return;
        }
        if let Some(rec) = self.get_mut(id) {
            rec.target = 8 * r.min(MAX_RADIUS);
        }
    }

    /// Sets intensity and colour.
    pub fn set_color(&mut self, id: LightId, i: u8, red: u8, green: u8, blue: u8) {
        if let Some(rec) = self.get_mut(id) {
            (rec.i, rec.r, rec.g, rec.b) = (i, red, green, blue);
        }
    }

    /// The current radius in whole sub-tiles.
    pub fn radius(&self, id: LightId) -> Option<i32> {
        self.get(id).map(|r| r.radius >> 3)
    }

    /// Removes a record at once.
    pub fn remove(&mut self, id: LightId) -> Result<LightRecord, LightError> {
        let pos = self
            .records
            .iter()
            .position(|(i, _)| *i == id)
            .ok_or(LightError::NotInList)?;
        Ok(self.records.remove(pos).1)
    }

    /// Lets a record fade out: it shrinks one sub-tile per drawn frame and
    /// leaves the list when its radius reaches 0. Cached records cannot
    /// die.
    pub fn die(&mut self, id: LightId) -> Result<(), LightError> {
        let rec = self.get_mut(id).ok_or(LightError::NotInList)?;
        if rec.kind == LightKind::Cached {
            return Err(LightError::DieOnCached);
        }
        rec.dying = true;
        rec.target = 0;
        Ok(())
    }

    /// One drawn frame: every record from head to tail is updated (it
    /// follows its owner and steps its radius; a faded-out dying record
    /// leaves the list) and adds its light to `map` at quality `q`. Order
    /// matters: colours mix by the intensity already in a cell, so the
    /// newest light is mixed in first.
    pub fn frame(
        &mut self,
        map: &mut LightMap,
        q: u8,
        world: &impl LightWorld,
    ) -> Result<(), LightError> {
        let colored = self.colored;
        let mut idx = 0;
        while idx < self.records.len() {
            let rec = &mut self.records[idx].1;
            // Follow the owner: a live record takes its owner's position.
            if !rec.dying {
                if let Some(owner) = rec.owner() {
                    if let Some((px, py)) = world.owner_position(&owner) {
                        rec.x = unit_light_pos(px);
                        rec.y = unit_light_pos(py);
                    }
                }
            }
            // Step the radius 8 toward the target; a cached shape is rebuilt.
            if rec.radius != rec.target {
                if rec.kind == LightKind::Cached {
                    rec.free_cache();
                }
                rec.radius += if rec.radius < rec.target { 8 } else { -8 };
            }
            // A radius past 255 falls back to 248.
            if rec.radius > 255 {
                rec.radius = 248;
            }
            // A dying record that has faded out leaves the list.
            if rec.dying && rec.radius < 1 {
                self.records.remove(idx);
                continue;
            }
            // Add the light to the map by the record's kind and the quality.
            match rec.kind {
                LightKind::Shadowed if q > 1 => contribute::shadowed(map, rec, colored),
                LightKind::Cached => contribute::cached(map, rec, world, colored)?,
                _ => {
                    let local = rec.owner().is_some_and(|o| world.is_local_player(&o));
                    contribute::plain(map, rec, q, local, colored);
                }
            }
            idx += 1;
        }
        Ok(())
    }

    /// A new active room appeared on the client. A cached record's shadow
    /// grid was built from the rooms that existed then, so every cached
    /// record whose light reaches into `new` (one of the four points at
    /// its radius east, west, south or north of its owner lies in it) marks
    /// its grid invalid; the next frame rebuilds it in place. Records whose
    /// owner stands in `new` itself are left alone.
    pub fn room_created(&mut self, new: RoomId, world: &impl LightRooms) -> Result<(), LightError> {
        for (_, rec) in self.records.iter_mut() {
            if rec.kind != LightKind::Cached {
                continue;
            }
            let owner = rec.owner().ok_or(LightError::Fatal0x591)?;
            if world.owner_position(&owner).is_none() {
                return Err(LightError::Fatal0x591);
            }
            let room = world
                .owner_room(&owner)
                .ok_or(LightError::OwnerWithoutRoom)?;
            if room == new {
                continue;
            }
            let (ux, uy) = world.owner_subtile(&owner).ok_or(LightError::Fatal0x591)?;
            let m = rec.radius >> 3;
            let probes = [(ux + m, uy), (ux - m, uy), (ux, uy + m), (ux, uy - m)];
            if probes
                .iter()
                .any(|&(px, py)| world.cell_room(room, px, py) == Some(new))
            {
                rec.cache_valid = false;
            }
        }
        Ok(())
    }
}
