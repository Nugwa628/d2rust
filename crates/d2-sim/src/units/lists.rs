//! Unit identity (GUIDs) and the server's lists: game unit hash lists, act
//! room lists, room unit lists, room update queues and the client list.
//! The per-unit timer lists live in
//! [`crate::tick::timer`] beside the queue that owns their timers.
//!
//! Every list is an intrusive singly linked list kept by index, with the
//! base game's insertion rules, so iteration order is the base game's. The
//! primitives `*_first` / `*_next` let callers walk a list with the
//! iteration-while-modifying discipline each consumer needs:
//! read the next link before or after the body as stated there.

use thiserror::Error;

mod alloc;

/// Hash buckets per unit class: `GUID & 0x7F`.
pub const HASH_BUCKETS: usize = 128;

/// Number of acts (`game +0xBC + 4·act`).
pub const ACTS: usize = 5;

/// Unit type at unit +0x00.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum UnitType {
    Player = 0,
    Monster = 1,
    Object = 2,
    Missile = 3,
    Item = 4,
    /// Tile (warp).
    Tile = 5,
}

impl UnitType {
    /// All types in index order.
    pub const ALL: [UnitType; 6] = [
        UnitType::Player,
        UnitType::Monster,
        UnitType::Object,
        UnitType::Missile,
        UnitType::Item,
        UnitType::Tile,
    ];

    /// The type number (unit +0x00).
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Hash list index (table, D2MOO `GAME_RemapUnitTypeToListIndex`):
    /// missile and item swap places. `None` for tiles, which have a single
    /// list.
    pub const fn hash_list(self) -> Option<usize> {
        match self {
            UnitType::Player => Some(0),
            UnitType::Monster => Some(1),
            UnitType::Object => Some(2),
            UnitType::Item => Some(3),
            UnitType::Missile => Some(4),
            UnitType::Tile => None,
        }
    }
}

/// A server unit, by slot. Stable while the unit lives; a slot is reused
/// after the unit is removed. Not a GUID: GUIDs are [`UnitEntry::guid`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitId(pub u32);

/// A room (DRLG active room record), by slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoomId(pub u32);

/// A client record, by slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub u32);

/// List bookkeeping errors. The base game treats the first as fatal; the
/// others are misuse of the D2RUST API.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ListError {
    /// Hash insert of a GUID already in the bucket (fatal error).
    #[error("duplicate GUID {guid} for unit type {ty:?}")]
    DuplicateGuid { ty: UnitType, guid: u32 },
    #[error("unknown unit {0:?}")]
    UnknownUnit(UnitId),
    #[error("unknown room {0:?}")]
    UnknownRoom(RoomId),
    #[error("unknown client {0:?}")]
    UnknownClient(ClientId),
    #[error("act {0} does not exist")]
    UnknownAct(u8),
    #[error("unit {0:?} is already in a room")]
    AlreadyInRoom(UnitId),
}

/// One GUID counter per unit type (game +0x90 + 4·type).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuidCounters([u32; 6]);

impl GuidCounters {
    /// Next GUID of `ty`. `counter + 1`, with
    /// 0xFFFFFFFF replaced by 1.
    pub fn alloc(&mut self, ty: UnitType) -> u32 {
        let counter = &mut self.0[ty.index()];
        let mut next = counter.wrapping_add(1);
        if next == u32::MAX {
            next = 1;
        }
        *counter = next;
        next
    }

    /// The last GUID handed out for `ty` (0 before the first).
    pub fn get(&self, ty: UnitType) -> u32 {
        self.0[ty.index()]
    }

    /// Overwrites a counter (game loading, tests).
    pub fn set(&mut self, ty: UnitType, value: u32) {
        self.0[ty.index()] = value;
    }
}

/// A slot arena with LIFO slot reuse. Slot numbers never affect an
/// outcome: every order comes from the lists' links.
#[derive(Clone, Debug)]
struct Arena<T> {
    slots: Vec<Option<T>>,
    free: Vec<u32>,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Arena<T> {
    fn insert(&mut self, value: T) -> u32 {
        match self.free.pop() {
            Some(i) => {
                self.slots[i as usize] = Some(value);
                i
            }
            None => {
                self.slots.push(Some(value));
                (self.slots.len() - 1) as u32
            }
        }
    }

    fn remove(&mut self, i: u32) -> Option<T> {
        let v = self.slots.get_mut(i as usize)?.take()?;
        self.free.push(i);
        Some(v)
    }

    fn get(&self, i: u32) -> Option<&T> {
        self.slots.get(i as usize)?.as_ref()
    }

    fn get_mut(&mut self, i: u32) -> Option<&mut T> {
        self.slots.get_mut(i as usize)?.as_mut()
    }
}

/// The list fields of one unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitEntry {
    /// Unit type (+0x00).
    pub ty: UnitType,
    /// GUID (+0x0C).
    pub guid: u32,
    /// Counts toward its room's allied count: players and good-
    /// aligned monsters. Set by the creator (alignment: monster).
    pub allied: bool,
    room: Option<RoomId>,
    /// Hash bucket link (+0xE4).
    hash_next: Option<UnitId>,
    /// Room unit list link (+0xE8).
    room_next: Option<UnitId>,
    /// Update queue link (+0xE0).
    update_next: Option<UnitId>,
    /// In its room's update queue (+0xC4 bit 0x2000).
    queued: bool,
}

impl UnitEntry {
    /// The room the unit is in.
    pub fn room(&self) -> Option<RoomId> {
        self.room
    }

    /// Whether the unit is in its room's update queue (flag 0x2000).
    pub fn is_queued(&self) -> bool {
        self.queued
    }
}

/// The list fields of one room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomEntry {
    /// The act the room belongs to.
    pub act: u8,
    /// Room +0x34 bit 0: populated.
    pub populated: bool,
    /// Room +0x34 bit 1: its units marked active.
    pub units_active: bool,
    /// Room +0x34 bit 2: the update queue ignores units here.
    pub no_update: bool,
    /// Adjacent active rooms including this one (room +0x00 / +0x24).
    /// Order owned by the DRLG (unit-order open question 2);
    /// written by DRLG code.
    pub adjacent: Vec<RoomId>,
    active: bool,
    /// Act room list link (+0x7C).
    act_next: Option<RoomId>,
    /// Unit list head (+0x74).
    units_head: Option<UnitId>,
    /// Update queue head (+0x1C).
    update_head: Option<UnitId>,
    allied_count: u32,
    /// Room +0x38..+0x44: the GUIDs of the room's last four dead units,
    /// slot order; 0 before a
    /// slot is written (the room record is zeroed).
    pub dead_guids: [u32; 4],
    /// Room byte +0x14: the next slot of [`Self::dead_guids`] (0–3).
    pub dead_next: u8,
}

impl RoomEntry {
    /// The GUID goes
    /// to slot byte +0x14, which then advances modulo 4.
    pub fn record_dead(&mut self, guid: u32) {
        self.dead_guids[usize::from(self.dead_next & 3)] = guid;
        self.dead_next = (self.dead_next + 1) & 3;
    }

    /// Whether the room is in its act's room list.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Players and good-aligned monsters in the room.
    pub fn allied_count(&self) -> u32 {
        self.allied_count
    }
}

/// The list fields of one act.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActEntry {
    /// Act +0x00: some room has pending unit updates (cleared by
    /// step 6).
    pub pending_updates: bool,
    /// Act +0x54: a room was activated (cleared by the tick).
    pub pending_rooms: bool,
    /// Act +0x58: some room has unit-removal records.
    /// Set by the unit-removal message path (not specified yet).
    pub pending_removals: bool,
    /// Act +0x04: the environment record,
    /// created with the act.
    pub environment: crate::world::environment::Environment,
    /// The act is built for a client (game +0xBC + 4a non-null in
    /// the base game: the join builds it when its slot is empty). D2RUST
    /// keeps the act entry from
    /// creation; tick step 1 advances only built acts.
    pub built: bool,
    /// Room list head (+0x10).
    rooms_head: Option<RoomId>,
}

/// Client record states (client +0x04).
pub mod client_state {
    /// Game created, S→C 0x00 sent.
    pub const LOADING: u32 = 1;
    /// The join built the client's act and sent 0x03.
    pub const ACT_LOADED: u32 = 2;
    /// Joining: waiting for its room.
    pub const JOINING: u32 = 3;
    /// In game.
    pub const IN_GAME: u32 = 4;
    /// Changing act: waiting for its room.
    pub const CHANGING_ACT: u32 = 5;
}

/// The list fields of one client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientEntry {
    /// Client state (+0x04, [`client_state`]).
    pub state: u32,
    /// The client's player unit.
    pub player: Option<UnitId>,
    /// The client's room (compared with the player's room).
    pub room: Option<RoomId>,
    /// Client counter +0x1B0, +1 per per-client update.
    pub update_count: u32,
    /// The bits the game set in the client status word (+0x0A)
    /// since the join: the bit setter
    /// (0x08, dead) and the progress setter (bits 8–12). The loaded word itself
    /// is the save's; a save ORs these into it (no code clears a bit).
    pub status_set: u16,
    /// The loaded Iron Golem item waiting for the game entry (+0x484, the
    /// item's GUID, −1 none): set by the
    /// save load's `kf` reader, consumed by the game entry.
    pub golem_pending: bool,
    /// The active view extras of the client's room in a game wider than
    /// 4:3 (empty otherwise), set at each room change: the per-client
    /// update sends their units after the adjacency array's.
    pub view_rooms: Vec<RoomId>,
    /// Client list link (+0x4A8).
    next: Option<ClientId>,
}

/// All lists of one game.
#[derive(Clone, Debug)]
pub struct UnitLists {
    /// GUID counters.
    pub guids: GuidCounters,
    units: Arena<UnitEntry>,
    /// Hash list heads by list index and bucket.
    hash: [[Option<UnitId>; HASH_BUCKETS]; 5],
    /// Tile list head (game +0x1B20).
    tiles: Option<UnitId>,
    rooms: Arena<RoomEntry>,
    acts: [Option<ActEntry>; ACTS],
    clients: Arena<ClientEntry>,
    /// Client list head (game +0x88).
    client_head: Option<ClientId>,
}

impl Default for UnitLists {
    fn default() -> Self {
        Self {
            guids: GuidCounters::default(),
            units: Arena::default(),
            hash: [[None; HASH_BUCKETS]; 5],
            tiles: None,
            rooms: Arena::default(),
            acts: Default::default(),
            clients: Arena::default(),
            client_head: None,
        }
    }
}

impl UnitLists {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- units -------------------------------------------------------

    pub fn unit(&self, id: UnitId) -> Option<&UnitEntry> {
        self.units.get(id.0)
    }

    pub fn unit_mut(&mut self, id: UnitId) -> Option<&mut UnitEntry> {
        self.units.get_mut(id.0)
    }

    /// Sets the unit's allied flag after creation (a later alignment
    /// change, e.g.), keeping its room's
    /// allied count in step. An unknown unit: nothing.
    pub fn set_allied(&mut self, id: UnitId, allied: bool) {
        let Some(e) = self.units.get_mut(id.0) else {
            return;
        };
        if e.allied == allied {
            return;
        }
        e.allied = allied;
        if let Some(room) = e.room {
            let r = self.r(room);
            if allied {
                r.allied_count += 1;
            } else {
                r.allied_count = r.allied_count.wrapping_sub(1);
            }
        }
    }

    fn unit_ok(&self, id: UnitId) -> Result<&UnitEntry, ListError> {
        self.unit(id).ok_or(ListError::UnknownUnit(id))
    }

    fn u(&mut self, id: UnitId) -> &mut UnitEntry {
        self.units.get_mut(id.0).expect("linked unit exists")
    }

    /// `SUNIT_Add`: place the unit in `room`,
    /// insert it in its hash list, queue it for update. The
    /// GUID comes from [`GuidCounters::alloc`] or, for restored units, the
    /// caller. [`Self::reserve_unit`] then [`Self::link_unit`].
    pub fn add_unit(
        &mut self,
        ty: UnitType,
        guid: u32,
        room: Option<RoomId>,
        allied: bool,
    ) -> Result<UnitId, ListError> {
        if let Some(r) = room {
            self.room(r).ok_or(ListError::UnknownRoom(r))?;
        }
        // Checked first so a fatal duplicate leaves no partial insert.
        self.check_guid_free(ty, guid)?;
        let id = self.reserve_unit(ty, guid, allied);
        self.link_unit(id, room)?;
        Ok(id)
    }

    /// The fatal duplicate GUID of `SUNIT_Add`, tested alone.
    pub fn check_guid_free(&self, ty: UnitType, guid: u32) -> Result<(), ListError> {
        if self.hash_bucket_of(ty, guid).any(|(_, e)| e.guid == guid) {
            return Err(ListError::DuplicateGuid { ty, guid });
        }
        Ok(())
    }

    /// A unit entry in no list (the record
    /// between allocation and `SUNIT_Add`): no room, not in its hash
    /// list or the update queue, so hash lookups do not find it. It owns
    /// timers like any entry. [`Self::link_unit`] adds it.
    pub fn reserve_unit(&mut self, ty: UnitType, guid: u32, allied: bool) -> UnitId {
        UnitId(self.units.insert(UnitEntry {
            ty,
            guid,
            allied,
            room: None,
            hash_next: None,
            room_next: None,
            update_next: None,
            queued: false,
        }))
    }

    /// The list part of `SUNIT_Add` for a [`Self::reserve_unit`] entry:
    /// room insert, hash insert, update queue.
    pub fn link_unit(&mut self, id: UnitId, room: Option<RoomId>) -> Result<(), ListError> {
        let (ty, guid) = {
            let e = self.unit_ok(id)?;
            (e.ty, e.guid)
        };
        if let Some(r) = room {
            self.room(r).ok_or(ListError::UnknownRoom(r))?;
        }
        self.check_guid_free(ty, guid)?;
        if let Some(r) = room {
            self.room_insert(id, r)?;
        }
        self.hash_insert(id);
        self.queue_update(id)?;
        Ok(())
    }

    /// The list part of unit removal: room list and
    /// update queue unlink, then hash unlink, then the slot is freed.
    /// Timers: [`crate::game::Game::remove_unit`].
    pub fn remove_unit(&mut self, id: UnitId) -> Result<UnitEntry, ListError> {
        self.unit_ok(id)?;
        self.room_remove(id)?;
        self.hash_remove(id);
        Ok(self.units.remove(id.0).expect("checked above"))
    }

    // ---- hash lists ----------------------------------------------

    fn hash_head(&mut self, ty: UnitType, guid: u32) -> &mut Option<UnitId> {
        match ty.hash_list() {
            Some(l) => &mut self.hash[l][guid as usize & (HASH_BUCKETS - 1)],
            None => &mut self.tiles,
        }
    }

    fn hash_head_ref(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        match ty.hash_list() {
            Some(l) => self.hash[l][guid as usize & (HASH_BUCKETS - 1)],
            None => self.tiles,
        }
    }

    fn hash_bucket_of(
        &self,
        ty: UnitType,
        guid: u32,
    ) -> impl Iterator<Item = (UnitId, &UnitEntry)> + '_ {
        let mut cur = self.hash_head_ref(ty, guid);
        std::iter::from_fn(move || {
            let id = cur?;
            let e = self.unit(id)?;
            cur = e.hash_next;
            Some((id, e))
        })
    }

    /// Before the first unit whose GUID is ≤ the new
    /// one, so buckets stay sorted by GUID, descending.
    fn hash_insert(&mut self, id: UnitId) {
        let (ty, guid) = {
            let e = self.u(id);
            (e.ty, e.guid)
        };
        let mut prev: Option<UnitId> = None;
        let mut cur = *self.hash_head(ty, guid);
        while let Some(c) = cur {
            let e = self.u(c);
            if e.guid <= guid {
                break;
            }
            prev = Some(c);
            cur = e.hash_next;
        }
        self.u(id).hash_next = cur;
        match prev {
            Some(p) => self.u(p).hash_next = Some(id),
            None => *self.hash_head(ty, guid) = Some(id),
        }
    }

    /// Unlink in place.
    fn hash_remove(&mut self, id: UnitId) {
        let (ty, guid, next) = {
            let e = self.u(id);
            (e.ty, e.guid, e.hash_next)
        };
        let mut prev: Option<UnitId> = None;
        let mut cur = *self.hash_head(ty, guid);
        while let Some(c) = cur {
            if c == id {
                match prev {
                    Some(p) => self.u(p).hash_next = next,
                    None => *self.hash_head(ty, guid) = next,
                }
                self.u(id).hash_next = None;
                return;
            }
            prev = Some(c);
            cur = self.u(c).hash_next;
        }
    }

    /// The first unit of `ty` with `guid`.
    pub fn find_unit(&self, ty: UnitType, guid: u32) -> Option<UnitId> {
        self.hash_bucket_of(ty, guid)
            .find(|(_, e)| e.guid == guid)
            .map(|(id, _)| id)
    }

    /// The GUIDs in hash bucket `bucket` of `ty`, head first (tiles: the
    /// single list, `bucket` ignored). A bucket past 127 is empty.
    pub fn hash_bucket(&self, ty: UnitType, bucket: usize) -> Vec<UnitId> {
        let mut out = Vec::new();
        let mut cur = match ty.hash_list() {
            Some(l) => self.hash[l].get(bucket).copied().flatten(),
            None => self.tiles,
        };
        while let Some(id) = cur {
            out.push(id);
            cur = self.unit(id).and_then(|e| e.hash_next);
        }
        out
    }

    fn first_from_bucket(&self, ty: UnitType, from: usize) -> Option<UnitId> {
        match ty.hash_list() {
            Some(l) => self.hash[l][from.min(HASH_BUCKETS)..]
                .iter()
                .find_map(|h| *h),
            None if from == 0 => self.tiles,
            None => None,
        }
    }

    /// First unit of a full iteration of `ty`: buckets 0..127 in
    /// index order, each from its head.
    pub fn first_of_type(&self, ty: UnitType) -> Option<UnitId> {
        self.first_from_bucket(ty, 0)
    }

    /// The unit after `id` in a full iteration of its type: its
    /// bucket successor, else the head of the next non-empty bucket.
    /// `None` also when `id` no longer exists (a callback must not remove
    /// the unit it is given).
    pub fn next_of_type(&self, id: UnitId) -> Option<UnitId> {
        let e = self.unit(id)?;
        if e.hash_next.is_some() {
            return e.hash_next;
        }
        let bucket = e.guid as usize & (HASH_BUCKETS - 1);
        e.ty.hash_list()?;
        self.first_from_bucket(e.ty, bucket + 1)
    }

    /// All units of `ty` in iteration order.
    pub fn units_of_type(&self, ty: UnitType) -> Vec<UnitId> {
        let mut out = Vec::new();
        let mut cur = self.first_of_type(ty);
        while let Some(id) = cur {
            out.push(id);
            cur = self.next_of_type(id);
        }
        out
    }

    // ---- acts and act room lists ----------------------------------

    /// Creates act `act` (0..4) if it does not exist.
    pub fn ensure_act(&mut self, act: u8) -> Result<(), ListError> {
        let slot = self
            .acts
            .get_mut(act as usize)
            .ok_or(ListError::UnknownAct(act))?;
        slot.get_or_insert_with(ActEntry::default);
        Ok(())
    }

    pub fn act(&self, act: u8) -> Option<&ActEntry> {
        self.acts.get(act as usize)?.as_ref()
    }

    pub fn act_mut(&mut self, act: u8) -> Option<&mut ActEntry> {
        self.acts.get_mut(act as usize)?.as_mut()
    }

    pub fn room(&self, id: RoomId) -> Option<&RoomEntry> {
        self.rooms.get(id.0)
    }

    pub fn room_mut(&mut self, id: RoomId) -> Option<&mut RoomEntry> {
        self.rooms.get_mut(id.0)
    }

    fn r(&mut self, id: RoomId) -> &mut RoomEntry {
        self.rooms.get_mut(id.0).expect("linked room exists")
    }

    /// A new, inactive room in `act` (DRLG room creation, not specified
    /// here).
    pub fn create_room(&mut self, act: u8) -> Result<RoomId, ListError> {
        self.act(act).ok_or(ListError::UnknownAct(act))?;
        Ok(RoomId(self.rooms.insert(RoomEntry {
            act,
            populated: false,
            units_active: false,
            no_update: false,
            adjacent: Vec::new(),
            active: false,
            act_next: None,
            units_head: None,
            update_head: None,
            allied_count: 0,
            dead_guids: [0; 4],
            dead_next: 0,
        })))
    }

    /// Frees a room record (DRLG; step 10), deactivating it
    /// first if active. Units still in it are unlinked from it (in
    /// list order) so no unit keeps the id of a freed (and later reused)
    /// room slot. TODO: the base game also gives each such
    /// unit flag 0x800000 and a path update (unit).
    pub fn free_room(&mut self, id: RoomId) -> Result<RoomEntry, ListError> {
        if self.room(id).ok_or(ListError::UnknownRoom(id))?.active {
            self.deactivate_room(id)?;
        }
        while let Some(u) = self.room_unit_first(id) {
            self.room_remove(u)?;
        }
        Ok(self.rooms.remove(id.0).expect("checked above"))
    }

    /// Prepend to the act's room list and set the
    /// act's pending-room flag. The room seed is set by
    /// the caller. Activating an active room does nothing.
    pub fn activate_room(&mut self, id: RoomId) -> Result<(), ListError> {
        let room = self.room(id).ok_or(ListError::UnknownRoom(id))?;
        if room.active {
            return Ok(());
        }
        let act_no = room.act;
        let act = self.act_mut(act_no).ok_or(ListError::UnknownAct(act_no))?;
        let head = act.rooms_head.replace(id);
        act.pending_rooms = true;
        let room = self.r(id);
        room.act_next = head;
        room.active = true;
        Ok(())
    }

    /// Unlink in place.
    pub fn deactivate_room(&mut self, id: RoomId) -> Result<(), ListError> {
        let room = self.room(id).ok_or(ListError::UnknownRoom(id))?;
        if !room.active {
            return Ok(());
        }
        let (act_no, next) = (room.act, room.act_next);
        let mut prev: Option<RoomId> = None;
        let mut cur = self.act(act_no).and_then(|a| a.rooms_head);
        while let Some(c) = cur {
            if c == id {
                break;
            }
            prev = Some(c);
            cur = self.r(c).act_next;
        }
        match prev {
            Some(p) => self.r(p).act_next = next,
            None => {
                if let Some(a) = self.act_mut(act_no) {
                    a.rooms_head = next;
                }
            }
        }
        let room = self.r(id);
        room.act_next = None;
        room.active = false;
        Ok(())
    }

    /// Head of the act's room list (newest-activated first).
    pub fn room_first(&self, act: u8) -> Option<RoomId> {
        self.act(act)?.rooms_head
    }

    /// Next room in the act's room list.
    pub fn room_next(&self, id: RoomId) -> Option<RoomId> {
        self.room(id)?.act_next
    }

    /// The act's active rooms in list order.
    pub fn active_rooms(&self, act: u8) -> Vec<RoomId> {
        let mut out = Vec::new();
        let mut cur = self.room_first(act);
        while let Some(r) = cur {
            out.push(r);
            cur = self.room_next(r);
        }
        out
    }

    // ---- room unit lists ------------------------------------------

    /// Prepend to the room's unit list, raise the
    /// allied count, then queue for update.
    pub fn room_insert(&mut self, unit: UnitId, room: RoomId) -> Result<(), ListError> {
        if self.unit_ok(unit)?.room.is_some() {
            return Err(ListError::AlreadyInRoom(unit));
        }
        let r = self.room_mut(room).ok_or(ListError::UnknownRoom(room))?;
        let head = r.units_head.replace(unit);
        let allied = self.units.get(unit.0).is_some_and(|e| e.allied);
        if allied {
            self.r(room).allied_count += 1;
        }
        let e = self.u(unit);
        e.room = Some(room);
        e.room_next = head;
        self.queue_update(unit)
    }

    /// Unlink from the room list (in place), remove
    /// from the update queue, lower the allied count. A unit without a
    /// room is left as is.
    pub fn room_remove(&mut self, unit: UnitId) -> Result<(), ListError> {
        let e = self.unit_ok(unit)?;
        let Some(room) = e.room else {
            return Ok(());
        };
        let (next, allied) = (e.room_next, e.allied);
        let mut prev: Option<UnitId> = None;
        let mut cur = self.r(room).units_head;
        while let Some(c) = cur {
            if c == unit {
                break;
            }
            prev = Some(c);
            cur = self.u(c).room_next;
        }
        match prev {
            Some(p) => self.u(p).room_next = next,
            None => self.r(room).units_head = next,
        }
        self.unqueue_update(unit)?;
        if allied {
            let r = self.r(room);
            r.allied_count = r.allied_count.wrapping_sub(1);
        }
        let e = self.u(unit);
        e.room = None;
        e.room_next = None;
        Ok(())
    }

    /// Room change by the path code: remove from the
    /// old room, prepend to the new one, queue for update.
    pub fn change_room(&mut self, unit: UnitId, room: RoomId) -> Result<(), ListError> {
        self.room(room).ok_or(ListError::UnknownRoom(room))?;
        self.room_remove(unit)?;
        self.room_insert(unit, room)
    }

    /// Head of the room's unit list (newest arrival first).
    pub fn room_unit_first(&self, room: RoomId) -> Option<UnitId> {
        self.room(room)?.units_head
    }

    /// Next unit in its room's unit list.
    pub fn room_unit_next(&self, unit: UnitId) -> Option<UnitId> {
        self.unit(unit)?.room_next
    }

    /// The room's units in list order.
    pub fn room_units(&self, room: RoomId) -> Vec<UnitId> {
        let mut out = Vec::new();
        let mut cur = self.room_unit_first(room);
        while let Some(u) = cur {
            out.push(u);
            cur = self.room_unit_next(u);
        }
        out
    }

    // ---- room update queues ---------------------------------------

    /// Prepend to the room's update queue unless the
    /// unit has no room, the room has flag bit 2, or the unit is queued;
    /// then set the act's pending-update flag.
    pub fn queue_update(&mut self, unit: UnitId) -> Result<(), ListError> {
        let e = self.unit_ok(unit)?;
        let Some(room) = e.room else {
            return Ok(());
        };
        if e.queued {
            return Ok(());
        }
        let r = self.r(room);
        if r.no_update {
            return Ok(());
        }
        let head = r.update_head.replace(unit);
        let act = r.act;
        let e = self.u(unit);
        e.update_next = head;
        e.queued = true;
        if let Some(a) = self.act_mut(act) {
            a.pending_updates = true;
        }
        Ok(())
    }

    /// Unlink from the update queue and clear the
    /// flag; nothing when not queued.
    pub fn unqueue_update(&mut self, unit: UnitId) -> Result<(), ListError> {
        let e = self.unit_ok(unit)?;
        let (Some(room), true) = (e.room, e.queued) else {
            return Ok(());
        };
        let next = e.update_next;
        let mut prev: Option<UnitId> = None;
        let mut cur = self.r(room).update_head;
        while let Some(c) = cur {
            if c == unit {
                break;
            }
            prev = Some(c);
            cur = self.u(c).update_next;
        }
        match prev {
            Some(p) => self.u(p).update_next = next,
            None => self.r(room).update_head = next,
        }
        let e = self.u(unit);
        e.update_next = None;
        e.queued = false;
        Ok(())
    }

    /// Unlink every unit of the room's update queue
    /// and clear their flags.
    pub fn clear_update_queue(&mut self, room: RoomId) -> Result<(), ListError> {
        let r = self.room_mut(room).ok_or(ListError::UnknownRoom(room))?;
        let mut cur = r.update_head.take();
        while let Some(u) = cur {
            let e = self.u(u);
            cur = e.update_next.take();
            e.queued = false;
        }
        Ok(())
    }

    /// Head of the room's update queue (most recently queued first).
    pub fn update_first(&self, room: RoomId) -> Option<UnitId> {
        self.room(room)?.update_head
    }

    /// Next unit in its room's update queue.
    pub fn update_next(&self, unit: UnitId) -> Option<UnitId> {
        self.unit(unit)?.update_next
    }

    /// The room's update queue in order.
    pub fn update_queue(&self, room: RoomId) -> Vec<UnitId> {
        let mut out = Vec::new();
        let mut cur = self.update_first(room);
        while let Some(u) = cur {
            out.push(u);
            cur = self.update_next(u);
        }
        out
    }

    // ---- client list ----------------------------------------------

    /// A joining client is prepended.
    pub fn add_client(
        &mut self,
        player: Option<UnitId>,
        room: Option<RoomId>,
        state: u32,
    ) -> ClientId {
        let id = ClientId(self.clients.insert(ClientEntry {
            state,
            player,
            room,
            update_count: 0,
            status_set: 0,
            golem_pending: false,
            view_rooms: Vec::new(),
            next: self.client_head,
        }));
        self.client_head = Some(id);
        id
    }

    /// Unlink.
    pub fn remove_client(&mut self, id: ClientId) -> Result<ClientEntry, ListError> {
        let next = self.client(id).ok_or(ListError::UnknownClient(id))?.next;
        let mut prev: Option<ClientId> = None;
        let mut cur = self.client_head;
        while let Some(c) = cur {
            if c == id {
                break;
            }
            prev = Some(c);
            cur = self.client(c).and_then(|e| e.next);
        }
        match prev {
            Some(p) => self.clients.get_mut(p.0).expect("linked").next = next,
            None => self.client_head = next,
        }
        Ok(self.clients.remove(id.0).expect("checked above"))
    }

    pub fn client(&self, id: ClientId) -> Option<&ClientEntry> {
        self.clients.get(id.0)
    }

    pub fn client_mut(&mut self, id: ClientId) -> Option<&mut ClientEntry> {
        self.clients.get_mut(id.0)
    }

    /// The bit setter with (client, bits, 1) on the client of
    /// `player`: `bits` set in its status word
    /// ([`ClientEntry::status_set`]). A player without a client: nothing.
    pub fn set_player_status(&mut self, player: UnitId, bits: u16) {
        for c in self.clients() {
            if let Some(e) = self.client_mut(c).filter(|e| e.player == Some(player)) {
                e.status_set |= bits;
            }
        }
    }

    /// Head of the client list (newest client first).
    pub fn client_first(&self) -> Option<ClientId> {
        self.client_head
    }

    pub fn client_next(&self, id: ClientId) -> Option<ClientId> {
        self.client(id)?.next
    }

    /// The clients in list order.
    pub fn clients(&self) -> Vec<ClientId> {
        let mut out = Vec::new();
        let mut cur = self.client_first();
        while let Some(c) = cur {
            out.push(c);
            cur = self.client_next(c);
        }
        out
    }
}

#[cfg(test)]
mod gaps_tests;
#[cfg(test)]
mod tests;
