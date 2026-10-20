//! Waypoints: index ↔ level, the per-difficulty record, its save section,
//! waypoint objects (init 17, operate 23), C→S 0x49 and travel.
//!
//! Objects, player fields, interaction, warps and room search belong to
//! other modules; they are reached through [`WaypointWorld`]. The arrival
//! list is game state owned by the object control; the caller keeps an
//! [`ArrivalList`] per game and passes it in.

use d2_data::tables::{Levels, Objects};

use crate::units::{RoomId, UnitId};

#[cfg(test)]
mod gaps_tests;
#[cfg(test)]
mod tests;

/// Marks a level with no waypoint.
pub const NO_WAYPOINT: u8 = 255;
/// Size of the (byte, mask) flag table: waypoint indexes below this are valid.
pub const FLAG_TABLE_SIZE: u32 = 0x70;
/// Record magic.
pub const MAGIC: u16 = 0x0102;
/// Operate function of waypoint objects.
pub const OPERATE_FN: u8 = 23;
/// Init function of waypoint objects.
pub const INIT_FN: u8 = 17;
/// Town levels.
pub const TOWNS: [u32; 5] = [1, 40, 75, 103, 109];
/// Non-town levels that also travel with tile code 13.
pub const TILE13_LEVELS: [u32; 6] = [46, 74, 133, 134, 135, 136];
/// The tile code of a waypoint room.
pub const TILE_WAYPOINT: u8 = 13;
/// Sorceress class id (she may use a waypoint from farther away).
pub const SORCERESS: u8 = 1;
/// Sound event of the hostile-delay refusal.
pub const SOUND_REFUSED: u8 = 0x13;
/// Interaction unit type of a waypoint menu.
pub const INTERACT_OBJECT: u8 = 2;
/// Save section size.
pub const SAVE_SECTION_LEN: usize = 80;

/// Invalid states, returned as errors instead of aborting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WaypointError {
    /// Index ≥ 0x70 at a bit test or set.
    #[error("waypoint index {0} is outside the 0x70-entry flag table")]
    IndexOutOfRange(u32),
    /// A record magic other than 0x0102, 0x0101, 0x0000.
    #[error("waypoint record magic {0:#06x}")]
    BadMagic(u16),
    /// Player data missing in the 0x49 validation.
    #[error("player data missing")]
    NoPlayerData,
}

// ------------------------------------------------------- index ↔ level

/// Waypoint index ↔ level, derived from the loaded `levels` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaypointMap {
    /// Per level id (= record number): (`Waypoint` byte, `Act` byte).
    levels: Vec<(u8, u8)>,
}

impl WaypointMap {
    /// From the loaded (and patched) `levels` records.
    pub fn new(levels: &[Levels]) -> Self {
        Self {
            levels: levels.iter().map(|l| (l.waypoint, l.act)).collect(),
        }
    }

    /// From (`Waypoint`, `Act`) per level id; for tests and fakes.
    pub fn from_pairs(levels: Vec<(u8, u8)>) -> Self {
        Self { levels }
    }

    /// The levels record count.
    pub fn level_count(&self) -> u32 {
        self.levels.len() as u32
    }

    /// The waypoint index of `level`, or `None`.
    pub fn index_of_level(&self, level: u32) -> Option<u8> {
        let &(wp, _) = self.levels.get(level as usize)?;
        (wp != NO_WAYPOINT).then_some(wp)
    }

    /// The first level id 1, 2, … count−1 with index `wp`.
    pub fn level_of_index(&self, wp: u32) -> Option<u32> {
        if wp >= u32::from(NO_WAYPOINT) {
            return None;
        }
        (1..self.level_count()).find(|&l| self.levels[l as usize].0 as u32 == wp)
    }

    /// The `Act` byte of `level` (0-based), if the record exists.
    pub fn act(&self, level: u32) -> Option<u8> {
        self.levels.get(level as usize).map(|&(_, act)| act)
    }

    /// The waypoint rows this table derives, in
    /// index order: (index, level, act, town, tile code).
    pub fn rows(&self) -> Vec<(u8, u32, u8, bool, u8)> {
        let mut rows: Vec<_> = (1..self.level_count())
            .filter_map(|l| {
                let wp = self.index_of_level(l)?;
                (self.level_of_index(wp.into()) == Some(l)).then(|| {
                    let act = self.act(l).unwrap_or(0);
                    (wp, l, act, is_town(l), tile_code(l))
                })
            })
            .collect();
        rows.sort_by_key(|r| r.0);
        rows
    }
}

/// Whether `level` is a town.
pub fn is_town(level: u32) -> bool {
    TOWNS.contains(&level)
}

/// The tile code a travel to `level` uses: 13 for towns and 46, 74, 133–136; else 0.
pub fn tile_code(level: u32) -> u8 {
    if is_town(level) || TILE13_LEVELS.contains(&level) {
        TILE_WAYPOINT
    } else {
        0
    }
}

// ------------------------------------------------------------ record

/// One difficulty's 16-byte waypoint record: magic word, then bit n
/// at byte 2 + n/8, mask 1 << (n mod 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointRecord(pub [u8; 16]);

impl Default for WaypointRecord {
    fn default() -> Self {
        Self::allocate()
    }
}

/// (byte, mask) of index `n`.
fn bit_of(n: u32) -> Result<(usize, u8), WaypointError> {
    if n >= FLAG_TABLE_SIZE {
        return Err(WaypointError::IndexOutOfRange(n));
    }
    Ok((2 + (n / 8) as usize, 1 << (n % 8)))
}

impl WaypointRecord {
    /// A new record: zero, magic 0x0102, bit 0 set (the first town).
    pub fn allocate() -> Self {
        let mut r = [0u8; 16];
        r[..2].copy_from_slice(&MAGIC.to_le_bytes());
        r[2] = 1;
        Self(r)
    }

    /// The magic word.
    pub fn magic(&self) -> u16 {
        u16::from_le_bytes([self.0[0], self.0[1]])
    }

    /// Whether index `n` is known.
    pub fn test(&self, n: u32) -> Result<bool, WaypointError> {
        let (byte, mask) = bit_of(n)?;
        Ok(self.0[byte] & mask != 0)
    }

    /// Marks index `n` known. A bit is never cleared.
    pub fn set(&mut self, n: u32) -> Result<(), WaypointError> {
        let (byte, mask) = bit_of(n)?;
        self.0[byte] |= mask;
        Ok(())
    }

    /// The load copy. Magic 0x0000
    /// or 0x0101 wipes the record to zero (magic ends 0x0000); then bit 0
    /// is set.
    pub fn load_copy(src: &[u8; 16]) -> Result<Self, WaypointError> {
        let mut r = Self(*src);
        match r.magic() {
            MAGIC => {}
            0x0000 | 0x0101 => r.0 = [0; 16],
            m => return Err(WaypointError::BadMagic(m)),
        }
        r.0[2] |= 1;
        Ok(r)
    }

    /// The out copy. Normalises **this** record's
    /// magic 0x0000 / 0x0101 to 0x0102, then copies.
    pub fn out_copy(&mut self) -> Result<[u8; 16], WaypointError> {
        match self.magic() {
            MAGIC => {}
            0x0000 | 0x0101 => self.0[..2].copy_from_slice(&MAGIC.to_le_bytes()),
            m => return Err(WaypointError::BadMagic(m)),
        }
        Ok(self.0)
    }
}

/// A player's three records, one per difficulty. Owned by the player
/// unit (provider: the units group).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WaypointRecords(pub [WaypointRecord; 3]);

impl WaypointRecords {
    /// The record of difficulty `d`.
    pub fn get_mut(&mut self, d: u8) -> &mut WaypointRecord {
        &mut self.0[usize::from(d)]
    }
}

// ------------------------------------------------------------ save section

/// Reading the save section failed. The caller maps it to its own error
/// code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SaveError {
    #[error("fewer than 80 bytes left")]
    Short,
    #[error("section header is not \"WS\"")]
    Header,
    #[error("record magic {0:#06x}")]
    Magic(u16),
}

/// Writes the 80-byte section. Normalises the records'
/// magic through the out copy.
pub fn write_section(records: &mut WaypointRecords) -> Result<[u8; 80], WaypointError> {
    let mut s = [0u8; SAVE_SECTION_LEN];
    s[0..2].copy_from_slice(b"WS");
    s[2..6].copy_from_slice(&1u32.to_le_bytes());
    s[6..8].copy_from_slice(&0x50u16.to_le_bytes());
    for (d, r) in records.0.iter_mut().enumerate() {
        let o = 8 + 24 * d;
        s[o..o + 16].copy_from_slice(&r.out_copy()?);
    }
    Ok(s)
}

/// Reads the section at the start of `bytes`. The u32 at +2, the size at +6
/// and the trailing bytes of each difficulty are not read.
pub fn read_section(bytes: &[u8]) -> Result<WaypointRecords, SaveError> {
    if bytes.len() < SAVE_SECTION_LEN {
        return Err(SaveError::Short);
    }
    if &bytes[0..2] != b"WS" {
        return Err(SaveError::Header);
    }
    let mut out = WaypointRecords::default();
    for d in 0..3 {
        let o = 8 + 24 * d;
        let src: [u8; 16] = bytes[o..o + 16].try_into().expect("16 bytes");
        out.0[d] = WaypointRecord::load_copy(&src).map_err(|e| match e {
            WaypointError::BadMagic(m) => SaveError::Magic(m),
            _ => unreachable!("load_copy only fails on the magic"),
        })?;
    }
    Ok(out)
}

// ------------------------------------------------------- objects and travel

/// The `objects.txt` fields waypoint code reads, by object class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectClass {
    pub operate_fn: u8,
    pub init_fn: u8,
    /// `FrameCnt1`, already × 256.
    pub frame_cnt1: u32,
}

/// Waypoint data from the loaded tables: the index map and the object
/// class fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaypointData {
    pub map: WaypointMap,
    pub objects: Vec<ObjectClass>,
}

impl WaypointData {
    /// From the loaded `levels` and `objects` records.
    pub fn new(levels: &[Levels], objects: &[Objects]) -> Self {
        Self {
            map: WaypointMap::new(levels),
            objects: objects
                .iter()
                .map(|o| ObjectClass {
                    operate_fn: o.operatefn,
                    init_fn: o.initfn,
                    frame_cnt1: o.framecnt1,
                })
                .collect(),
        }
    }

    /// Classes with operate function 23 and init function 17.
    pub fn waypoint_classes(&self) -> Vec<u16> {
        (0..self.objects.len())
            .filter(|&c| {
                let o = &self.objects[c];
                o.operate_fn == OPERATE_FN && o.init_fn == INIT_FN
            })
            .map(|c| c as u16)
            .collect()
    }
}

/// What the waypoint code reads of an object unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectFacts {
    pub guid: u32,
    /// `objects.txt` class id.
    pub class: u16,
    pub mode: u8,
    pub room: Option<RoomId>,
    /// Level id of its room.
    pub level: Option<u32>,
    /// Position in subtiles (from its static path).
    pub x: i32,
    pub y: i32,
}

/// What the waypoint code reads of a player unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerFacts {
    pub guid: u32,
    /// Character class id.
    pub class: u8,
    pub room: Option<RoomId>,
    /// Level id of its room.
    pub level: Option<u32>,
    pub x: i32,
    pub y: i32,
}

/// A room's rectangle in subtiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl RoomRect {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// The seam to the rest of the game. Expected providers in brackets.
pub trait WaypointWorld {
    /// Current frame (`Game::frame`).
    fn frame(&self) -> i32;
    /// The game's tick rate: durations stated in base ticks are
    /// converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> crate::tick::TickRate;
    /// Game difficulty (game).
    fn difficulty(&self) -> u8;
    /// The player's records (units group). `None`:
    /// player data missing.
    fn records(&mut self, player: UnitId) -> Option<&mut WaypointRecords>;
    /// The object unit with this GUID (unit lists).
    fn object(&self, guid: u32) -> Option<(UnitId, ObjectFacts)>;
    /// Player position, room, class (units group).
    fn player(&self, player: UnitId) -> PlayerFacts;
    /// Room rectangle (DRLG).
    fn room_rect(&self, room: RoomId) -> RoomRect;
    /// Object mode change, including the animation-setup random draw
    /// (objects).
    fn set_object_mode(&mut self, object: UnitId, mode: u8);
    /// Schedule ENDANIM (timer event type 1) on the object
    /// at `frame` (`Game::schedule_event`).
    fn schedule_endanim(&mut self, object: UnitId, frame: i32);
    /// Whether the player is busy: interact info active, item on the
    /// cursor, or a player-data busy flag set (units / interaction).
    fn player_busy(&self, player: UnitId) -> bool;
    /// Set the interact info; ignored if already active.
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32);
    /// Reset the interact info.
    fn reset_interact(&mut self, player: UnitId);
    /// GUID of the player's interact unit, if any.
    fn interact_guid(&self, player: UnitId) -> Option<u32>;
    /// Whether the player is still inside the 10 s hostile-action delay
    /// (host clock; never true in single player).
    fn hostile_delay(&self, player: UnitId) -> bool;
    /// Attach a sound event to the player (units).
    fn attach_sound(&mut self, player: UnitId, event: u8);
    /// Queue a message to the player's client (server transport).
    fn send(&mut self, player: UnitId, msg: &[u8]);
    /// Warp to `level` with `tile_code` (act change or spawn
    /// search + placement; DRLG / level change).
    fn warp(&mut self, player: UnitId, level: u32, tile_code: u8);
    /// The spawn room for (level, tile code), without the
    /// free-coordinate step (DRLG).
    fn spawn_room(&mut self, level: u32, tile_code: u8) -> Option<RoomId>;
    /// Player mode 2 at its own position, with no skill (player modes;
    /// can draw a random number).
    fn set_player_mode_arrival(&mut self, player: UnitId);
}

/// One arrival record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrivalNode {
    pub room: Option<RoomId>,
    pub x: i32,
    pub y: i32,
}

/// The object control's arrival list, head first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArrivalList(pub Vec<ArrivalNode>);

/// Result codes of the 0x49 handler.
pub mod result {
    pub const OK: u32 = 0;
    pub const NOT_FOUND: u32 = 1;
    pub const REFUSED: u32 = 2;
    pub const BAD: u32 = 3;
}

impl WaypointData {
    /// Init function 17 at object creation.
    pub fn init_object<W: WaypointWorld>(
        &self,
        world: &mut W,
        arrivals: &mut ArrivalList,
        object: UnitId,
        facts: &ObjectFacts,
    ) {
        let rect = facts.room.map(|r| world.room_rect(r));
        let hit = arrivals.0.iter().any(|n| {
            // Pointer equality: two null rooms match too.
            n.room == facts.room
                || (facts.room.is_some() && rect.is_some_and(|r| r.contains(n.x, n.y)))
        });
        if hit {
            if facts.mode == 0 {
                world.set_object_mode(object, 1);
                // The opening animation, one frame per base tick.
                let n = world
                    .tick_rate()
                    .ticks((self.frame_cnt(facts.class) >> 8) as i32);
                world.schedule_endanim(object, world.frame() + n);
            }
            // The matching node is freed and the head set to null, so every
            // other node is dropped too.
            arrivals.0.clear();
        } else if facts.level.is_some_and(is_town) {
            world.set_object_mode(object, 2);
        }
    }

    fn frame_cnt(&self, class: u16) -> u32 {
        self.objects
            .get(usize::from(class))
            .map_or(0, |o| o.frame_cnt1)
    }

    /// Operate function 23. Always returns 1.
    pub fn operate<W: WaypointWorld>(
        &self,
        world: &mut W,
        object: UnitId,
        facts: &ObjectFacts,
        player: UnitId,
    ) -> Result<u32, WaypointError> {
        let d = world.difficulty();
        if let Some(wp) = facts.level.and_then(|l| self.map.index_of_level(l)) {
            if let Some(records) = world.records(player) {
                records.get_mut(d).set(wp.into())?;
            }
        }
        match facts.mode {
            0 => {
                world.set_object_mode(object, 1);
                // The opening animation, one frame per base tick.
                let n = world
                    .tick_rate()
                    .ticks((self.frame_cnt(facts.class) >> 8) as i32);
                world.schedule_endanim(object, world.frame() + n + 1);
            }
            1 | 2 => {
                if world.player_busy(player) {
                    return Ok(1);
                }
                let Some(records) = world.records(player) else {
                    return Ok(1);
                };
                let rec = records.get_mut(d).out_copy()?;
                world.send(player, &menu_message(facts.guid, &rec));
                world.set_interact(player, INTERACT_OBJECT, facts.guid);
            }
            _ => {}
        }
        Ok(1)
    }

    /// The 0x49 validation; returns the result code.
    pub fn validate<W: WaypointWorld>(
        &self,
        world: &mut W,
        player: UnitId,
        wp: u32,
        level: u32,
    ) -> Result<u32, WaypointError> {
        let Some((_, obj)) = world.object(wp) else {
            return Ok(result::NOT_FOUND);
        };
        let p = world.player(player);
        // TODO(waypoints): a player or object without a room compares as
        // "no act"; the game's behaviour for a missing room is not known.
        let p_act = p.level.and_then(|l| self.map.act(l));
        let o_act = obj.level.and_then(|l| self.map.act(l));
        if p_act != o_act {
            return Ok(result::REFUSED);
        }
        let max = if p.class == SORCERESS { 22 } else { 10 };
        if (p.x - obj.x).abs() > max || (p.y - obj.y).abs() > max {
            return Ok(result::NOT_FOUND);
        }
        if level == 0 {
            return Ok(result::OK);
        }
        if level >= self.map.level_count() {
            return Ok(result::BAD);
        }
        let d = world.difficulty();
        let records = world.records(player).ok_or(WaypointError::NoPlayerData)?;
        let Some(idx) = self.map.index_of_level(level) else {
            return Ok(result::BAD);
        };
        if !records.get_mut(d).test(idx.into())? {
            return Ok(result::REFUSED);
        }
        Ok(result::OK)
    }

    /// C→S 0x49 TakeOrCloseWp. `msg` is the whole
    /// message. Returns the result code.
    pub fn take_or_close<W: WaypointWorld>(
        &self,
        world: &mut W,
        arrivals: &mut ArrivalList,
        player: UnitId,
        msg: &[u8],
    ) -> Result<u32, WaypointError> {
        if msg.len() != 9 {
            return Ok(result::BAD);
        }
        let wp = u32::from_le_bytes(msg[1..5].try_into().expect("4 bytes"));
        let level = u32::from(u16::from_le_bytes([msg[5], msg[6]]));
        if world.hostile_delay(player) {
            world.attach_sound(player, SOUND_REFUSED);
            world.reset_interact(player);
            return Ok(result::NOT_FOUND);
        }
        let r = self.validate(world, player, wp, level)?;
        if r != result::OK {
            if world.interact_guid(player) == Some(wp) {
                world.reset_interact(player);
            }
            return Ok(r);
        }
        self.travel(world, arrivals, player, wp, level)?;
        Ok(result::OK)
    }

    /// Travel to `level` through the waypoint object `wp`.
    pub fn travel<W: WaypointWorld>(
        &self,
        world: &mut W,
        arrivals: &mut ArrivalList,
        player: UnitId,
        wp: u32,
        level: u32,
    ) -> Result<(), WaypointError> {
        let Some((_, obj)) = world.object(wp) else {
            return Ok(());
        };
        // The class's operate function must be 23 (classes past the table
        // and every other class fail).
        match self.objects.get(usize::from(obj.class)) {
            Some(o) if o.operate_fn == OPERATE_FN => {}
            _ => return Ok(()),
        }
        world.reset_interact(player);
        // Level 0 or the object's own level is the whole "close".
        if level == 0 || obj.level == Some(level) {
            return Ok(());
        }
        let d = world.difficulty();
        let Some(idx) = self.map.index_of_level(level) else {
            return Ok(());
        };
        let Some(records) = world.records(player) else {
            return Ok(());
        };
        if !records.get_mut(d).test(idx.into())? {
            return Ok(());
        }
        let tile = tile_code(level);
        world.warp(player, level, tile);
        let p = world.player(player);
        if p.room.is_some() && p.room == world.spawn_room(level, tile) {
            world.set_player_mode_arrival(player);
            world.send(player, &arrival_message(p.guid, p.x, p.y));
        }
        arrivals.0.insert(
            0,
            ArrivalNode {
                room: p.room,
                x: p.x,
                y: p.y,
            },
        );
        Ok(())
    }
}

/// S→C 0x63 WaypointMenu (21 bytes).
pub fn menu_message(guid: u32, record: &[u8; 16]) -> [u8; 21] {
    let mut m = [0u8; 21];
    m[0] = 0x63;
    m[1..5].copy_from_slice(&guid.to_le_bytes());
    m[5..].copy_from_slice(record);
    m
}

/// The arrival S→C 0x0D: unit type 0, GUID, 1, x + 3, y + 3, 0, 0 (13
/// bytes; the two trailing bytes are always zero).
pub fn arrival_message(guid: u32, x: i32, y: i32) -> [u8; 13] {
    let mut m = [0u8; 13];
    m[0] = 0x0D;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m[6] = 1;
    // Subtile coordinates travel as u16 (truncation is the wire format).
    m[7..9].copy_from_slice(&((x + 3) as u16).to_le_bytes());
    m[9..11].copy_from_slice(&((y + 3) as u16).to_le_bytes());
    m
}

#[cfg(test)]
mod mutant_tests;
