//! Character save (`.d2s`, version 0x60): the byte layout, the checksum, the
//! section framing and the loader's format checks. `d2-server` character
//! storage reads and writes saves through this module; what a load does to
//! the game (quest normalisation, gold limits, hireling restore, item
//! placement) belongs there, not here.
//!
//! A save is a fixed 335-byte [`Header`] followed by the [`Body`] sections in
//! a fixed order: quests, waypoints, NPC flags, stats, skills, the player's
//! items, the corpse, and in an expansion game the hireling's items and the
//! Iron Golem. A freshly created character is the bare header with the
//! "new" status bit set; the body is written the first time the character
//! leaves a game.
//!
//! The model holds every byte a save carries, including the fields the
//! loader never reads, so reading a file and writing the model back
//! reproduces it byte for byte. Item records stay opaque byte entries: their
//! length comes from the item bit stream reader through [`SaveTables`].
//!
//! The parts, in file order: the header ([`Header`], [`check_header`]),
//! the body sections ([`Body`]), the stats bit field ([`write_stats`]), the
//! reader ([`read`], and [`clear_stale_new_flag`] for a save that kept its
//! new-character flag) and the writer ([`write`]). Legacy saves (versions below
//! 0x5C) are read by [`legacy`]; the 32 appearance bytes of the header are
//! built by [`appearance`].

pub mod appearance;
pub mod legacy;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_c2fmt;
#[cfg(test)]
mod tests_fitems;

/// File magic at the start of every save.
pub const MAGIC: u32 = 0xAA55_AA55;
/// The version D2RUST writes.
pub const VERSION: u32 = 0x60;
/// Lowest version the current loader takes; older saves go to [`legacy`].
pub const VERSION_MIN: u32 = 0x5C;
/// Header size: the whole file of a new character.
pub const HEADER_SIZE: usize = 0x14F;
/// The largest save: the writer refuses to grow past it and the loader
/// ignores any bytes beyond it.
pub const MAX_FILE: usize = 0x2000;

/// Section offsets and sizes. The first three sections have fixed sizes, so
/// their offsets are fixed too; everything from the stats on is variable.
pub const QUEST_OFFSET: usize = 0x14F;
pub const QUEST_SIZE: usize = 298;
pub const WAYPOINT_OFFSET: usize = 0x279;
pub const WAYPOINT_SIZE: usize = 80;
pub const NPC_OFFSET: usize = 0x2C9;
pub const NPC_SIZE: usize = 52;
pub const STATS_OFFSET: usize = 0x2FD;

/// Section markers and the fixed size and version words inside them. Read
/// as text, the markers are `Woo!` (quests), `WS` (waypoints), `gf` (stats),
/// `if` (skills), `JM` (item lists and records), `jf` (hireling items) and
/// `kf` (Iron Golem); tests and messages use those names.
pub const QUEST_MAGIC: u32 = 0x216F_6F57;
pub const QUEST_VERSION: u32 = 6;
pub const QUEST_SECTION_SIZE: u16 = 0x012A;
pub const WAYPOINT_MAGIC: u16 = 0x5357;
pub const WAYPOINT_VERSION: u32 = 1;
pub const WAYPOINT_SECTION_SIZE: u16 = 0x50;
pub const NPC_MAGIC: u16 = 0x7701;
pub const NPC_SECTION_SIZE: u16 = 0x34;
pub const STATS_MAGIC: u16 = 0x6667;
pub const SKILLS_MAGIC: u16 = 0x6669;
pub const ITEMS_MAGIC: u16 = 0x4D4A;
pub const HIRELING_MAGIC: u16 = 0x666A;
pub const GOLEM_MAGIC: u16 = 0x666B;
/// The 9-bit stat id that ends the stats bit field.
pub const STATS_END: u32 = 0x1FF;

/// Bits of the header's status word (+0x24).
pub mod status {
    /// A freshly created character: the file is the bare header.
    pub const NEW: u16 = 0x0001;
    pub const REALM: u16 = 0x0002;
    pub const HARDCORE: u16 = 0x0004;
    /// A hardcore character that died; it never loads again.
    pub const DEAD: u16 = 0x0008;
    pub const EXPANSION: u16 = 0x0020;
    pub const LADDER: u16 = 0x0040;

    /// Progression, bits 8–12: how many acts the character has completed,
    /// counted across the difficulties; it unlocks Nightmare and Hell.
    pub fn progression(s: u16) -> u8 {
        ((s >> 8) & 0x1F) as u8
    }

    /// Mask bit of Normal in [`difficulties`].
    pub const NORMAL_BIT: u8 = 1;
    /// Mask bit of Nightmare.
    pub const NIGHTMARE_BIT: u8 = 2;
    /// Mask bit of Hell.
    pub const HELL_BIT: u8 = 4;

    /// The difficulties a character has unlocked, as a mask (bit 0 Normal,
    /// bit 1 Nightmare, bit 2 Hell), from its status word. Normal is always
    /// open. Nightmare opens at progression 5 for an expansion character and 4
    /// for a classic one; Hell at 10 and 8. Character select and the online
    /// lobby both decide with this rule (the lobby: which games a character
    /// may create and join).
    pub fn difficulties(s: u16) -> u8 {
        let p = progression(s);
        let (nightmare, hell) = if s & EXPANSION != 0 { (5, 10) } else { (4, 8) };
        let mut mask = NORMAL_BIT;
        if p >= nightmare {
            mask |= NIGHTMARE_BIT;
        }
        if p >= hell {
            mask |= HELL_BIT;
        }
        mask
    }

    /// Whether difficulty `d` (0 Normal, 1 Nightmare, 2 Hell) is in the mask.
    pub fn difficulty_open(mask: u8, d: u8) -> bool {
        d < 3 && mask & (1 << d) != 0
    }
}

/// Item flag "in a store": cleared on every item a load creates.
pub const ITEM_FLAG_INSTORE: u32 = 0x2000;
/// Item flag "created by a load": set on every item a load creates, cleared
/// by the writer.
pub const ITEM_FLAG_LOADED: u32 = 0x80000;
/// Item record bit "alternate code": dropped from the stored flags when the
/// record is decoded.
pub const ITEM_FLAG_ALT_CODE: u32 = 0x200_0000;

/// The flags an item created from a save record carries: the stored flags
/// without the "loaded" and alternate-code bits (as the record decode leaves
/// them), then "loaded" set and "in a store" cleared.
pub fn item_flags_on_load(stored: u32) -> u32 {
    ((stored & !(ITEM_FLAG_LOADED | ITEM_FLAG_ALT_CODE)) | ITEM_FLAG_LOADED) & !ITEM_FLAG_INSTORE
}

/// The 16 appearance component bytes of a new character: component 1 for
/// the head, torso, legs, both arms and both shoulder pads (S1, S2); 0xFF
/// (nothing) for the hands, the shield and the unused parts 10–15 (see
/// [`appearance::part`]).
pub const STUB_COMPONENTS: [u8; 16] = [
    1, 1, 1, 1, 1, 0xFF, 0xFF, 0xFF, 1, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// Internal code → load result.
const RESULT: [u8; 27] = [
    0, 14, 9, 14, 9, 14, 14, 1, 24, 23, 21, 20, 19, 17, 18, 2, 3, 11, 4, 5, 7, 8, 10, 10, 14, 26,
    25,
];

/// The load result shown for an internal failure code; a code past the
/// table gives result 1 (a generic failure).
pub fn load_result(internal: u8) -> u8 {
    RESULT.get(usize::from(internal)).copied().unwrap_or(1)
}

/// The name field of a save file's header: +0x14 (16 bytes) in the current
/// format, +0x08 in a legacy one (see [`legacy`]), up to the
/// first NUL and at most 15 bytes; empty for a file shorter than the header.
pub fn header_name(bytes: &[u8]) -> &[u8] {
    let at = if bytes.len() >= 8 && (1..VERSION_MIN).contains(&u32_at(bytes, 4)) {
        0x08
    } else {
        0x14
    };
    let Some(field) = bytes.get(at..at + 15) else {
        return &[];
    };
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    &field[..end]
}

/// The status word of a save file's header (+0x24 u16, legacy +0x18);
/// `None` for a file shorter than the header.
pub fn header_status(bytes: &[u8]) -> Option<u16> {
    let at = if bytes.len() >= 8 && (1..VERSION_MIN).contains(&u32_at(bytes, 4)) {
        0x18
    } else {
        0x24
    };
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

/// The class byte of a save file's header (+0x28, legacy +0x22).
pub fn header_class(bytes: &[u8]) -> u8 {
    let at = if bytes.len() >= 8 && (1..VERSION_MIN).contains(&u32_at(bytes, 4)) {
        0x22
    } else {
        0x28
    };
    bytes.get(at).copied().unwrap_or(0)
}

/// The save checksum: rotate the running sum left by one bit, then add the
/// next byte. It is taken over the whole file with its own field (+0x0C)
/// zeroed.
pub fn checksum(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |s, &b| s.rotate_left(1).wrapping_add(u32::from(b)))
}

/// Why a file did not load.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum D2sError {
    /// Fewer than 8 bytes or a bad magic: load result 9.
    #[error("not a character save (fewer than 8 bytes or bad magic)")]
    NotASave,
    /// The legacy loader (version below 0x5C, see [`legacy`]) failed with
    /// this load result.
    #[error("legacy load result {result} ({what}) at offset {offset:#x}")]
    LegacyResult {
        result: u8,
        what: &'static str,
        offset: usize,
        /// The header had already created and announced the player unit.
        after_player: bool,
    },
    /// A legacy part D2RUST does not read yet (item records, hireling).
    #[error("legacy save part not implemented: {0}")]
    LegacyUnsupported(&'static str),
    /// A check or section reader failed with an internal failure code;
    /// [`load_result`] maps it to what the player sees.
    #[error("internal code {code} ({what}) at offset {offset:#x}")]
    Load {
        code: u8,
        what: &'static str,
        offset: usize,
    },
}

impl D2sError {
    /// The load result the player sees; `None` for a legacy part that is
    /// not read.
    pub fn result(&self) -> Option<u8> {
        match self {
            D2sError::NotASave => Some(9),
            D2sError::LegacyResult { result, .. } => Some(*result),
            D2sError::LegacyUnsupported(_) => None,
            D2sError::Load { code, .. } => Some(load_result(*code)),
        }
    }

    /// The internal failure code, when a check or section reader failed.
    pub fn internal(&self) -> Option<u8> {
        match self {
            D2sError::Load { code, .. } => Some(*code),
            _ => None,
        }
    }
}

/// A load failure with internal code `code`, found at byte `offset`.
fn fail(code: u8, what: &'static str, offset: usize) -> D2sError {
    D2sError::Load { code, what, offset }
}

/// Why the writer stopped.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The file would pass [`MAX_FILE`] bytes.
    #[error("save passes the {MAX_FILE}-byte buffer ({0} bytes)")]
    Overflow(usize),
    /// A class above 6 has no skill list to write.
    #[error("class {0} has no skill list (writer error 2)")]
    Class(u8),
    /// A model field cannot be stored in its section, or would make a file
    /// the loader refuses.
    #[error("{0}")]
    Model(&'static str),
}

// ---------------------------------------------------------------- header

/// One hotkey or mouse-skill slot: u16 code, u16 item index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    pub code: u16,
    pub item: u16,
}

impl Slot {
    /// "No skill": skill −1 stored as 0xFFFF.
    pub const NONE: Slot = Slot {
        code: 0xFFFF,
        item: 0,
    };

    /// Encodes skill `s` (−1: none), the left flag and a 1-based item
    /// index (0: none). `None` when s > 0x7FFF, which does not fit the code.
    pub fn encode(skill: i32, left: bool, item: u16) -> Option<Slot> {
        if skill > 0x7FFF {
            return None;
        }
        let code = (skill as u32 & 0xFFFF) as u16 | if left { 0x8000 } else { 0 };
        Some(Slot { code, item })
    }

    /// Decodes to (skill, left, item index); −1 for no skill / no item.
    pub fn decode(self) -> (i32, bool, i32) {
        if self.code == 0xFFFF {
            return (-1, false, -1);
        }
        let item = if self.item == 0 {
            -1
        } else {
            i32::from(self.item)
        };
        (i32::from(self.code & 0x0FFF), self.code >> 15 != 0, item)
    }
}

/// Hireling block at +0xAF.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hireling {
    /// 0x10000 = dead.
    pub flags: u32,
    pub seed: u32,
    /// Name id − the row's `NameFirst`.
    pub name_index: u16,
    /// Hireling row `Id`.
    pub id: u16,
    pub experience: u32,
    /// +0xBF, 16 bytes the writer leaves zero.
    pub rest: [u8; 16],
}

impl Hireling {
    pub const DEAD: u32 = 0x10000;

    /// No hireling when seed, name index and experience are all 0.
    pub fn is_present(&self) -> bool {
        self.seed != 0 || self.name_index != 0 || self.experience != 0
    }
}

/// The 335-byte header, every byte kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    /// +0x08 as read; the writer stores the real length.
    pub file_size: u32,
    /// +0x0C as read; the writer stores the checksum of the file.
    pub checksum: u32,
    /// +0x10: bit 0 = weapon switch; the writer leaves the other bits 0.
    pub weapon_switch: u32,
    /// +0x14, NUL-terminated; byte 15 is forced 0 on load.
    pub name: [u8; 16],
    pub status: u16,
    /// +0x26, written 0.
    pub unk26: u16,
    pub class: u8,
    /// +0x29 (0x10; the number of stat slots in the 0x5C–0x5E stats layout).
    pub stat_count: u8,
    /// +0x2A: the skills section's byte count (30 in current saves).
    pub skill_count: u8,
    pub level: u8,
    pub create_time: u32,
    pub save_time: u32,
    /// +0x34, written 0xFFFFFFFF.
    pub unk34: u32,
    pub hotkeys: [Slot; 16],
    /// Left, right, swap left, swap right (+0x78..+0x87).
    pub mouse: [Slot; 4],
    pub components: [u8; 16],
    pub colours: [u8; 16],
    /// Town per difficulty: act | 0x80 for the current one (+0xA8).
    pub towns: [u8; 3],
    pub map_seed: u32,
    pub hireling: Hireling,
    /// +0xCF: a client-side byte whose meaning is not known; kept as read.
    pub client_cf: u8,
    /// +0xD0..+0x14E, written 0.
    pub tail: [u8; 127],
}

impl Default for Header {
    fn default() -> Self {
        Header {
            version: VERSION,
            file_size: 0,
            checksum: 0,
            weapon_switch: 0,
            name: [0; 16],
            status: 0,
            unk26: 0,
            class: 0,
            stat_count: 0x10,
            skill_count: 30,
            level: 1,
            create_time: 0,
            save_time: 0,
            unk34: 0xFFFF_FFFF,
            hotkeys: [Slot::NONE; 16],
            mouse: [Slot::default(); 4],
            components: [0xFF; 16],
            colours: [0xFF; 16],
            towns: [0; 3],
            map_seed: 0,
            hireling: Hireling::default(),
            client_cf: 0,
            tail: [0; 127],
        }
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn arr<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    let mut a = [0u8; N];
    a.copy_from_slice(&b[at..at + N]);
    a
}

impl Header {
    /// The writer's pre-fill of the 32 appearance bytes +0x88..+0xA7: all
    /// 0xFF. The writer then lets each equipped item change its bytes; with
    /// no equipped item this is the saved value. The per-item mapping is
    /// [`appearance`].
    pub fn reset_appearance(&mut self) {
        self.components = [0xFF; 16];
        self.colours = [0xFF; 16];
    }

    /// The writer's appearance bytes: the 0xFF pre-fill, then the fill of
    /// [`appearance::fill`] from the items equipped at save time. The bytes the header held before
    /// (a loaded file's) never reach the result.
    pub fn rebuild_appearance(
        &mut self,
        eq: &appearance::Equipment,
        t: &appearance::AppearanceTables,
    ) {
        self.reset_appearance();
        appearance::fill(eq, t, &mut self.components, &mut self.colours);
    }

    /// The name bytes before the first NUL, at most 15.
    pub fn name_bytes(&self) -> &[u8] {
        let n = self.name[..15].iter().position(|&c| c == 0).unwrap_or(15);
        &self.name[..n]
    }

    /// Sets the name; `None` when it is longer than 15 bytes or holds a NUL.
    pub fn set_name(&mut self, name: &[u8]) -> Option<()> {
        if name.len() > 15 || name.contains(&0) {
            return None;
        }
        self.name = [0; 16];
        self.name[..name.len()].copy_from_slice(name);
        Some(())
    }

    /// Decodes the first 0x14F bytes of `b` (no checks).
    fn parse(b: &[u8]) -> Header {
        let slot = |at: usize| Slot {
            code: u16_at(b, at),
            item: u16_at(b, at + 2),
        };
        let mut hotkeys = [Slot::default(); 16];
        for (i, h) in hotkeys.iter_mut().enumerate() {
            *h = slot(0x38 + 4 * i);
        }
        let mut mouse = [Slot::default(); 4];
        for (i, m) in mouse.iter_mut().enumerate() {
            *m = slot(0x78 + 4 * i);
        }
        Header {
            version: u32_at(b, 0x04),
            file_size: u32_at(b, 0x08),
            checksum: u32_at(b, 0x0C),
            weapon_switch: u32_at(b, 0x10),
            name: arr(b, 0x14),
            status: u16_at(b, 0x24),
            unk26: u16_at(b, 0x26),
            class: b[0x28],
            stat_count: b[0x29],
            skill_count: b[0x2A],
            level: b[0x2B],
            create_time: u32_at(b, 0x2C),
            save_time: u32_at(b, 0x30),
            unk34: u32_at(b, 0x34),
            hotkeys,
            mouse,
            components: arr(b, 0x88),
            colours: arr(b, 0x98),
            towns: arr(b, 0xA8),
            map_seed: u32_at(b, 0xAB),
            hireling: Hireling {
                flags: u32_at(b, 0xAF),
                seed: u32_at(b, 0xB3),
                name_index: u16_at(b, 0xB7),
                id: u16_at(b, 0xB9),
                experience: u32_at(b, 0xBB),
                rest: arr(b, 0xBF),
            },
            client_cf: b[0xCF],
            tail: arr(b, 0xD0),
        }
    }

    /// The 0x14F header bytes; +0x08 and +0x0C as stored in the model.
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut b = [0u8; HEADER_SIZE];
        let mut put = |at: usize, v: &[u8]| b[at..at + v.len()].copy_from_slice(v);
        put(0x00, &MAGIC.to_le_bytes());
        put(0x04, &self.version.to_le_bytes());
        put(0x08, &self.file_size.to_le_bytes());
        put(0x0C, &self.checksum.to_le_bytes());
        put(0x10, &self.weapon_switch.to_le_bytes());
        put(0x14, &self.name);
        put(0x24, &self.status.to_le_bytes());
        put(0x26, &self.unk26.to_le_bytes());
        put(
            0x28,
            &[self.class, self.stat_count, self.skill_count, self.level],
        );
        put(0x2C, &self.create_time.to_le_bytes());
        put(0x30, &self.save_time.to_le_bytes());
        put(0x34, &self.unk34.to_le_bytes());
        for (i, s) in self.hotkeys.iter().chain(self.mouse.iter()).enumerate() {
            put(0x38 + 4 * i, &s.code.to_le_bytes());
            put(0x3A + 4 * i, &s.item.to_le_bytes());
        }
        put(0x88, &self.components);
        put(0x98, &self.colours);
        put(0xA8, &self.towns);
        put(0xAB, &self.map_seed.to_le_bytes());
        let h = &self.hireling;
        put(0xAF, &h.flags.to_le_bytes());
        put(0xB3, &h.seed.to_le_bytes());
        put(0xB7, &h.name_index.to_le_bytes());
        put(0xB9, &h.id.to_le_bytes());
        put(0xBB, &h.experience.to_le_bytes());
        put(0xBF, &h.rest);
        put(0xCF, &[self.client_cf]);
        put(0xD0, &self.tail);
        b
    }
}

/// What the game knows when it loads a file; [`check_header`] compares the header
/// against it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameContext {
    /// The client's name (no NUL); empty = no name (internal 3).
    pub client_name: Vec<u8>,
    pub expansion: bool,
    /// The game is a hardcore game.
    pub hardcore: bool,
    /// The game's difficulty (0 normal, 1 Nightmare, 2 Hell).
    pub difficulty: u8,
}

/// Checks the header's name and status word against the game: client name,
/// expansion, hardcore and difficulty. A ladder check needs the realm
/// service, which single player lacks; it is not implemented.
pub fn check_header(h: &Header, game: &GameContext) -> Result<(), D2sError> {
    // The name must be present and match the client's, ignoring case.
    if game.client_name.is_empty() {
        return Err(fail(3, "no client name", 0x14));
    }
    if !h.name_bytes().eq_ignore_ascii_case(&game.client_name) {
        return Err(fail(7, "name differs from the client's", 0x14));
    }
    // Expansion flag against the game type.
    let s = h.status;
    if s & status::EXPANSION != 0 && !game.expansion {
        return Err(fail(8, "expansion character, classic game", 0x24));
    }
    if s & status::EXPANSION == 0 && game.expansion {
        return Err(fail(9, "classic character, expansion game", 0x24));
    }
    // Hardcore flag against the game mode; a dead hardcore character never loads.
    if s & status::HARDCORE != 0 {
        if s & status::DEAD != 0 {
            return Err(fail(10, "dead hardcore character", 0x24));
        }
        if !game.hardcore {
            return Err(fail(11, "hardcore character, softcore game", 0x24));
        }
    } else if game.hardcore {
        return Err(fail(12, "softcore character, hardcore game", 0x24));
    }
    // Nightmare and Hell need enough progression.
    let p = status::progression(s);
    let exp = s & status::EXPANSION != 0;
    match game.difficulty {
        0 => {}
        1 if p < if exp { 5 } else { 4 } => {
            return Err(fail(13, "Nightmare not unlocked", 0x25));
        }
        d if d >= 2 && p < if exp { 10 } else { 8 } => {
            return Err(fail(14, "Hell not unlocked", 0x25));
        }
        _ => {}
    }
    Ok(())
}

// --------------------------------------------------------------- sections

/// Quest section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quests {
    /// +8: written 0x12A, never read.
    pub size: u16,
    /// Quest flag record per difficulty.
    pub records: [[u8; 96]; 3],
}

impl Default for Quests {
    fn default() -> Self {
        Quests {
            size: QUEST_SECTION_SIZE,
            records: [[0; 96]; 3],
        }
    }
}

/// Waypoint section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waypoints {
    /// +2: written 1, never read.
    pub version: u32,
    /// +6: written 0x50, never read.
    pub size: u16,
    /// Per difficulty: the 16-byte record, then 8 bytes written zero.
    pub records: [[u8; 16]; 3],
    pub pads: [[u8; 8]; 3],
}

impl Default for Waypoints {
    fn default() -> Self {
        // Record magic 0x0102, bit 0 set (the first waypoint is open).
        let rec = {
            let mut r = [0u8; 16];
            r[0] = 0x02;
            r[1] = 0x01;
            r[2] = 0x01;
            r
        };
        Waypoints {
            version: WAYPOINT_VERSION,
            size: WAYPOINT_SECTION_SIZE,
            records: [rec; 3],
            pads: [[0; 8]; 3],
        }
    }
}

impl Waypoints {
    /// Byte and mask of waypoint index n (< 0x70) in a record: byte 2 + n/8,
    /// bit n mod 8.
    pub fn bit(n: u8) -> Option<(usize, u8)> {
        (n < 0x70).then(|| (2 + usize::from(n / 8), 1 << (n % 8)))
    }
}

/// NPC flag section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Npcs {
    /// +2: written 0x34, never read.
    pub size: u16,
    /// Field A per difficulty (set by [`Npcs::set_intro_a`]).
    pub a: [[u8; 8]; 3],
    /// Field B per difficulty: NPC intro bits.
    pub b: [[u8; 8]; 3],
}

impl Default for Npcs {
    fn default() -> Self {
        Npcs {
            size: NPC_SECTION_SIZE,
            a: [[0; 8]; 3],
            b: [[0; 8]; 3],
        }
    }
}

impl Npcs {
    /// Field A setter: in difficulty `d`'s field A, sets the bit of the
    /// first class range that contains `class_id` (bit 0 when none does):
    /// [`npc_bit`]. `None`: difficulty out of range.
    pub fn set_intro_a(&mut self, d: usize, class_id: i32) -> Option<()> {
        let n = npc_bit(class_id);
        let f = self.a.get_mut(d)?;
        f[usize::from(n >> 3)] |= 1 << (n & 7);
        Some(())
    }
}

/// NPC class id → bit in field A; 0 when the class is in no range.
pub fn npc_bit(class_id: i32) -> u8 {
    const RANGES: [(i32, i32, u8); 18] = [
        (147, 147, 1),
        (148, 148, 2),
        (150, 150, 3),
        (155, 155, 4),
        (154, 154, 5),
        (265, 265, 6),
        (175, 178, 7),
        (202, 202, 11),
        (200, 200, 12),
        (210, 210, 13),
        (201, 201, 14),
        (198, 198, 15),
        (199, 199, 16),
        (244, 246, 17),
        (251, 257, 20),
        (264, 264, 27),
        (297, 297, 28),
        (511, 515, 29),
    ];
    if class_id == 520 {
        return 34;
    }
    RANGES
        .iter()
        .find(|&&(lo, hi, _)| (lo..=hi).contains(&class_id))
        .map_or(0, |&(lo, _, b)| b + (class_id - lo) as u8)
}

/// One base stat entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatEntry {
    pub id: u16,
    pub layer: u16,
    pub value: i32,
}

/// Stats section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stats {
    /// Version > 0x5E: the bit field, in file order.
    Bits(Vec<StatEntry>),
    /// Versions 0x5C–0x5E: mask bytes, then one u32 per consumed value
    /// `values` holds (stat id, value) for set bits < k.
    Mask {
        mask: Vec<u8>,
        values: Vec<(u16, u32)>,
    },
}

impl Stats {
    /// The stat entries the loader sets (layer 0 for the mask layout;
    /// clear mask bits set 0, which stores no entry).
    pub fn entries(&self) -> Vec<StatEntry> {
        match self {
            Stats::Bits(v) => v.clone(),
            Stats::Mask { values, .. } => values
                .iter()
                .filter(|&&(_, v)| v != 0)
                .map(|&(id, v)| StatEntry {
                    id,
                    layer: 0,
                    value: v as i32,
                })
                .collect(),
        }
    }
}

/// The itemstatcost save columns of one stat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatSave {
    /// `CSvBits`.
    pub bits: u8,
    /// `CSvParam`.
    pub param: u8,
    /// `CSvSigned`.
    pub signed: bool,
}

/// One item entry: the item's stream and its socketed children, as
/// written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemEntry {
    pub bytes: Vec<u8>,
}

impl ItemEntry {
    /// The stored flags of the record starting at byte `at` of the entry
    /// (the u32 after the `JM` marker: bits 16–47 of the record, so byte
    /// aligned). `None`: no `JM`
    /// marker there.
    pub fn record_flags(&self, at: usize) -> Option<u32> {
        let b = self.bytes.get(at..at + 6)?;
        (u16_at(b, 0) == ITEMS_MAGIC).then(|| u32_at(b, 2))
    }

    /// Stores `flags` in the record starting at byte `at`. `None`: no
    /// `JM` marker there.
    pub fn set_record_flags(&mut self, at: usize, flags: u32) -> Option<()> {
        self.record_flags(at)?;
        self.bytes[at + 2..at + 6].copy_from_slice(&flags.to_le_bytes());
        Some(())
    }
}

/// Corpse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Corpse {
    /// Never assigned by the game; D2RUST writes 0.
    pub unk: u32,
    pub x: u32,
    pub y: u32,
    pub items: Vec<ItemEntry>,
}

/// Iron Golem section.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Golem {
    /// The u8 g.
    pub flag: u8,
    /// Present when g ≠ 0.
    pub item: Option<ItemEntry>,
}

/// Everything after the header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Body {
    pub quests: Quests,
    pub waypoints: Waypoints,
    pub npcs: Npcs,
    pub stats: Stats,
    /// One base level per byte; the loader reads header +0x2A of them.
    pub skills: Vec<u8>,
    pub items: Vec<ItemEntry>,
    /// 0 or 1 corpse (the loader rejects 2+).
    pub corpses: Vec<Corpse>,
    /// `jf`: `None` = section absent (classic, or fewer than 2 bytes
    /// left); `Some(None)` = marker without a list.
    pub hireling_items: Option<Option<Vec<ItemEntry>>>,
    /// `kf`: `None` = section absent.
    pub golem: Option<Golem>,
    /// Bytes after the last section: never written by the game, not
    /// checked by the loader; kept for a lossless rewrite.
    pub trailing: Vec<u8>,
}

impl Body {
    /// The `jf` section the writer stores: a classic game writes none; an
    /// expansion game writes the marker, then, when the player has a
    /// hireling pet node, that hireling's item list (empty when it carries
    /// nothing). `hireling`: `None` = no hireling node; `Some(entries)` =
    /// its items in save order.
    pub fn set_hireling_items(&mut self, expansion: bool, hireling: Option<Vec<ItemEntry>>) {
        self.hireling_items = expansion.then_some(hireling);
    }
}

impl Default for Stats {
    fn default() -> Self {
        Stats::Bits(Vec::new())
    }
}

/// A character save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct D2s {
    pub header: Header,
    /// `None`: a new character, whose file is the 335-byte header alone.
    pub body: Option<Body>,
}

/// The game tables the reader and writer need.
pub trait SaveTables {
    /// The `itemstatcost` save columns of a stat; `None` when the id is not
    /// below the table's row count.
    fn stat_save(&self, id: u16) -> Option<StatSave>;
    /// The byte length of the item entry at the start of `buf` (the item's
    /// record plus its socketed children), or why it does not decode.
    fn item_entry_len(&self, buf: &[u8]) -> Result<usize, String>;
    /// Whether the loader restores this hireling: present and its row
    /// found. Default: present.
    fn hireling_restored(&self, h: &Hireling) -> bool {
        h.is_present()
    }
}

/// How to read a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadOptions {
    /// An expansion game: the hireling item and Iron Golem sections are read.
    pub expansion: bool,
    /// When set, [`check_header`] runs against it.
    pub game: Option<GameContext>,
}

// ------------------------------------------------------------- bit stream

/// LSB-first bit reader. Reads past the end yield 0 bits and set `over`.
struct Bits<'a> {
    buf: &'a [u8],
    pos: usize,
    over: bool,
}

impl<'a> Bits<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Bits {
            buf,
            pos: 0,
            over: false,
        }
    }

    fn read(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            let at = self.pos;
            self.pos += 1;
            let bit = match self.buf.get(at / 8) {
                Some(&b) => (b >> (at % 8)) & 1,
                None => {
                    self.over = true;
                    0
                }
            };
            v |= u32::from(bit) << i;
        }
        v
    }

    fn byte_len(&self) -> usize {
        self.pos.div_ceil(8)
    }
}

/// LSB-first bit writer.
#[derive(Default)]
struct BitOut {
    buf: Vec<u8>,
    pos: usize,
}

impl BitOut {
    fn put(&mut self, n: u32, v: u32) {
        for i in 0..n {
            if self.pos.is_multiple_of(8) {
                self.buf.push(0);
            }
            if (v >> i) & 1 != 0 {
                *self.buf.last_mut().expect("pushed") |= 1 << (self.pos % 8);
            }
            self.pos += 1;
        }
    }
}

fn sign_extend(v: u32, n: u32) -> i32 {
    if n == 0 || n >= 32 {
        return v as i32;
    }
    let shift = 32 - n;
    ((v << shift) as i32) >> shift
}

/// The value written for v in n bits: clamped to the column's range.
pub fn clamp_stat(v: i32, col: StatSave) -> u32 {
    let n = u32::from(col.bits);
    if n >= 32 {
        return v as u32;
    }
    if col.signed {
        let lo = -(1i64 << (n - 1));
        let hi = (1i64 << (n - 1)) - 1;
        (i64::from(v).clamp(lo, hi) as u32) & ((1u32 << n) - 1)
    } else {
        let max = (1u32 << n) - 1;
        if v <= 0 {
            0
        } else {
            (v as u32).min(max)
        }
    }
}

/// The `gf` section bytes for version > 0x5E.
pub fn write_stats(entries: &[StatEntry], t: &dyn SaveTables) -> Vec<u8> {
    let mut out = STATS_MAGIC.to_le_bytes().to_vec();
    let mut w = BitOut::default();
    for e in entries {
        let Some(col) = t.stat_save(e.id) else {
            continue;
        };
        if col.bits == 0 {
            continue;
        }
        w.put(9, u32::from(e.id));
        if col.param != 0 {
            w.put(u32::from(col.param), u32::from(e.layer));
        }
        w.put(u32::from(col.bits), clamp_stat(e.value, col));
    }
    w.put(9, STATS_END);
    out.extend_from_slice(&w.buf);
    out
}

// ------------------------------------------------------------------ read

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn left(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn rest(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }

    fn u16(&mut self) -> u16 {
        let v = u16_at(self.buf, self.pos);
        self.pos += 2;
        v
    }

    fn take(&mut self, n: usize) -> &'a [u8] {
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        s
    }

    /// u16 0x4D4A, u16 count, then `count` entries; any failure is internal
    /// `code`.
    fn item_list(
        &mut self,
        t: &dyn SaveTables,
        code: u8,
        what: &'static str,
    ) -> Result<Vec<ItemEntry>, D2sError> {
        if self.left() < 4 || u16_at(self.buf, self.pos) != ITEMS_MAGIC {
            return Err(fail(code, what, self.pos));
        }
        self.pos += 2;
        let count = self.u16();
        let mut items = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            items.push(self.item_entry(t, code, what)?);
        }
        Ok(items)
    }

    fn item_entry(
        &mut self,
        t: &dyn SaveTables,
        code: u8,
        what: &'static str,
    ) -> Result<ItemEntry, D2sError> {
        let at = self.pos;
        let n = t
            .item_entry_len(self.rest())
            .map_err(|_| fail(code, what, at))?;
        if n == 0 || n > self.left() {
            return Err(fail(code, what, at));
        }
        Ok(ItemEntry {
            bytes: self.take(n).to_vec(),
        })
    }
}

/// The skill levels a load had read when a later section failed (internal
/// codes 20–23: the game applies each section as it reads it, so the player
/// holds these levels when the join is refused). `None` when the sections up to the
/// skills do not read.
pub fn skills_before_failure(file: &[u8], t: &dyn SaveTables) -> Option<Vec<u8>> {
    let buf = &file[..file.len().min(MAX_FILE)];
    if buf.len() < HEADER_SIZE {
        return None;
    }
    let header = Header::parse(buf);
    let mut r = Reader {
        buf,
        pos: HEADER_SIZE,
    };
    read_prefix(&mut r, &header, t).ok().map(|p| p.4)
}

/// The repaired bytes of a save that kept the "new character" status flag
/// after its first full save: a current-format file, longer than the bare
/// header, with a good checksum and size and [`status::NEW`] set. [`read`]
/// refuses such a file (internal code 2: the flag is only valid on the
/// header-only stub the creation screen writes), but its body is a normal
/// saved character. The repair clears the flag and stores the size and
/// checksum again; everything else is the file's own bytes. `None` when the
/// file is not that case (a stub, a damaged file, a flag already clear), so
/// the caller keeps the original error.
pub fn clear_stale_new_flag(file: &[u8]) -> Option<Vec<u8>> {
    let buf = &file[..file.len().min(MAX_FILE)];
    if buf.len() <= HEADER_SIZE || u32_at(buf, 0) != MAGIC {
        return None;
    }
    if !(VERSION_MIN..=VERSION).contains(&u32_at(buf, 4))
        || buf.len() as u64 != u64::from(u32_at(buf, 0x08))
    {
        return None;
    }
    let mut zeroed = buf.to_vec();
    zeroed[0x0C..0x10].fill(0);
    if checksum(&zeroed) != u32_at(buf, 0x0C) {
        return None;
    }
    let status = u16_at(buf, 0x24);
    if status & status::NEW == 0 {
        return None;
    }
    let mut out = buf.to_vec();
    out[0x24..0x26].copy_from_slice(&(status & !status::NEW).to_le_bytes());
    finish(&mut out);
    Some(out)
}

/// Reads a file. The bytes past 0x2000 are cut first, as a read into the
/// game's 0x2000-byte buffer would.
pub fn read(file: &[u8], opts: &ReadOptions, t: &dyn SaveTables) -> Result<D2s, D2sError> {
    let buf = &file[..file.len().min(MAX_FILE)];
    // Magic; shorter than 8 bytes is not a save.
    if buf.len() < 8 || u32_at(buf, 0) != MAGIC {
        return Err(D2sError::NotASave);
    }
    let version = u32_at(buf, 4);
    if version < VERSION_MIN {
        return legacy::read(buf, opts);
    }
    // A current-format save has the whole header.
    if buf.len() < HEADER_SIZE {
        return Err(fail(4, "header too short", 0));
    }
    // Checksum with +0x0C zeroed, then the size.
    let stored = u32_at(buf, 0x0C);
    let mut zeroed = buf.to_vec();
    zeroed[0x0C..0x10].fill(0);
    if checksum(&zeroed) != stored {
        return Err(fail(6, "checksum", 0x0C));
    }
    if buf.len() as u64 != u64::from(u32_at(buf, 0x08)) {
        return Err(fail(5, "file size", 0x08));
    }
    // Version in the supported range.
    if !(VERSION_MIN..=VERSION).contains(&version) {
        return Err(fail(7, "version", 0x04));
    }
    let header = Header::parse(buf);
    // Name and status checks against the game, when one is given.
    if let Some(game) = &opts.game {
        check_header(&header, game)?;
    }
    // Class byte in range.
    if header.class > 7 {
        return Err(fail(4, "class", 0x28));
    }
    // A new-character stub is the bare header.
    if header.status & status::NEW != 0 {
        if buf.len() == HEADER_SIZE {
            return Ok(D2s { header, body: None });
        }
        return Err(fail(2, "new-character flag with more data", 0x24));
    }
    let mut r = Reader {
        buf,
        pos: HEADER_SIZE,
    };
    let body = read_body(&mut r, &header, opts, t)?;
    Ok(D2s {
        header,
        body: Some(body),
    })
}

/// The sections up to the skills: quests, waypoints, NPC flags,
/// stats and the skill levels.
fn read_prefix(
    r: &mut Reader<'_>,
    h: &Header,
    t: &dyn SaveTables,
) -> Result<(Quests, Waypoints, Npcs, Stats, Vec<u8>), D2sError> {
    // Quest section: size, magic and version.
    let at = r.pos;
    if r.left() < QUEST_SIZE
        || u32_at(r.buf, at) != QUEST_MAGIC
        || u32_at(r.buf, at + 4) != QUEST_VERSION
    {
        return Err(fail(15, "quest section", at));
    }
    let s = r.take(QUEST_SIZE);
    let quests = Quests {
        size: u16_at(s, 8),
        records: [arr(s, 10), arr(s, 106), arr(s, 202)],
    };
    // Waypoint section.
    let at = r.pos;
    if r.left() < WAYPOINT_SIZE || u16_at(r.buf, at) != WAYPOINT_MAGIC {
        return Err(fail(16, "waypoint section", at));
    }
    let s = r.take(WAYPOINT_SIZE);
    let mut waypoints = Waypoints {
        version: u32_at(s, 2),
        size: u16_at(s, 6),
        records: [[0; 16]; 3],
        pads: [[0; 8]; 3],
    };
    for d in 0..3 {
        let rec: [u8; 16] = arr(s, 8 + 24 * d);
        if !matches!(u16_at(&rec, 0), 0x0102 | 0x0101 | 0x0000) {
            return Err(fail(16, "waypoint record magic", at + 8 + 24 * d));
        }
        waypoints.records[d] = rec;
        waypoints.pads[d] = arr(s, 24 + 24 * d);
    }
    // NPC section.
    let at = r.pos;
    if r.left() < NPC_SIZE || u16_at(r.buf, at) != NPC_MAGIC {
        return Err(fail(17, "NPC section", at));
    }
    let s = r.take(NPC_SIZE);
    let npcs = Npcs {
        size: u16_at(s, 2),
        a: [arr(s, 4), arr(s, 12), arr(s, 20)],
        b: [arr(s, 0x1C), arr(s, 0x24), arr(s, 0x2C)],
    };
    // Stats.
    let stats = if h.version > 0x5E {
        read_stats_bits(r, t)?
    } else {
        read_stats_mask(r, h)?
    };
    // Skills: the header's byte count follows the marker. The game does not
    // check that +0x2A bytes remain; D2RUST rejects a short section with 19.
    let at = r.pos;
    let n = usize::from(h.skill_count);
    if r.left() < 2 || u16_at(r.buf, at) != SKILLS_MAGIC || r.left() < 2 + n {
        return Err(fail(19, "skills section", at));
    }
    r.pos += 2;
    let skills = r.take(n).to_vec();
    Ok((quests, waypoints, npcs, stats, skills))
}

fn read_body(
    r: &mut Reader<'_>,
    h: &Header,
    opts: &ReadOptions,
    t: &dyn SaveTables,
) -> Result<Body, D2sError> {
    let (quests, waypoints, npcs, stats, skills) = read_prefix(r, h, t)?;
    // The player item list; errors surface as 20.
    let items = r.item_list(t, 20, "player items")?;
    // Corpses (more than one is an error).
    let corpses = read_corpses(r, t)?;
    let mut body = Body {
        quests,
        waypoints,
        npcs,
        stats,
        skills,
        items,
        corpses,
        hireling_items: None,
        golem: None,
        trailing: Vec::new(),
    };
    if opts.expansion {
        // Hireling items.
        if r.left() >= 2 {
            let at = r.pos;
            if r.u16() != HIRELING_MAGIC {
                return Err(fail(22, "hireling marker", at));
            }
            let list = if t.hireling_restored(&h.hireling) {
                Some(r.item_list(t, 22, "hireling items")?)
            } else {
                None
            };
            body.hireling_items = Some(list);
        }
        // Iron Golem.
        if r.left() >= 2 {
            let at = r.pos;
            if r.u16() != GOLEM_MAGIC {
                return Err(fail(23, "golem marker", at));
            }
            // The game reads g past the end of a cut file; D2RUST rejects it
            // instead.
            if r.left() < 1 {
                return Err(fail(23, "golem flag", r.pos));
            }
            let flag = r.take(1)[0];
            let item = if flag != 0 {
                Some(r.item_entry(t, 23, "golem item")?)
            } else {
                None
            };
            body.golem = Some(Golem { flag, item });
        }
    }
    body.trailing = r.rest().to_vec();
    Ok(body)
}

fn read_stats_bits(r: &mut Reader<'_>, t: &dyn SaveTables) -> Result<Stats, D2sError> {
    // Stats marker.
    let at = r.pos;
    if r.left() < 2 || u16_at(r.buf, at) != STATS_MAGIC {
        return Err(fail(18, "stats marker", at));
    }
    let start = at + 2;
    let mut b = Bits::new(&r.buf[start..]);
    let mut out = Vec::new();
    loop {
        let id = b.read(9);
        if id > 0x1FE {
            break;
        }
        // Without a terminator the game would read on forever; D2RUST
        // rejects it with 18.
        if b.over {
            return Err(fail(18, "stats without terminator", start));
        }
        let id = id as u16;
        let col = match t.stat_save(id) {
            Some(c) if c.bits != 0 => c,
            _ => return Err(fail(18, "stat not saved", start + (b.pos - 9) / 8)),
        };
        let layer = if col.param != 0 {
            let p = u32::from(col.param);
            sign_extend(b.read(p), p) as u16
        } else {
            0
        };
        let n = u32::from(col.bits);
        let raw = b.read(n);
        let value = if n < 32 && col.signed {
            sign_extend(raw, n)
        } else {
            raw as i32
        };
        out.push(StatEntry { id, layer, value });
    }
    if b.over {
        return Err(fail(18, "stats without terminator", start));
    }
    r.pos = start + b.byte_len();
    Ok(Stats::Bits(out))
}

fn read_stats_mask(r: &mut Reader<'_>, h: &Header) -> Result<Stats, D2sError> {
    // Mask layout: mask bytes, then one u32 per set bit.
    let at = r.pos;
    let k = usize::from(h.stat_count);
    let m = k.div_ceil(8);
    if r.left() < 2 + m || u16_at(r.buf, at) != STATS_MAGIC {
        return Err(fail(18, "stats section", at));
    }
    let mask = r.buf[at + 2..at + 2 + m].to_vec();
    // The bound counts every set bit of the mask bytes, including bits at or
    // above the stat count.
    let set: usize = mask.iter().map(|b| b.count_ones() as usize).sum();
    if r.left() < 2 + m + 4 * set {
        return Err(fail(18, "stats values", at));
    }
    let mut cur = at + 2 + m;
    let mut values = Vec::new();
    for i in 0..k {
        if mask[i >> 3] & (1 << (i & 7)) != 0 {
            values.push((i as u16, u32_at(r.buf, cur)));
            cur += 4;
        }
    }
    r.pos = cur;
    Ok(Stats::Mask { mask, values })
}

fn read_corpses(r: &mut Reader<'_>, t: &dyn SaveTables) -> Result<Vec<Corpse>, D2sError> {
    let at = r.pos;
    if r.left() < 4 || u16_at(r.buf, at) != ITEMS_MAGIC {
        return Err(fail(21, "corpse section", at));
    }
    r.pos += 2;
    let n = r.u16();
    if n >= 2 {
        return Err(fail(21, "corpse count", at + 2));
    }
    let mut out = Vec::new();
    for _ in 0..n {
        if r.left() < 12 {
            return Err(fail(21, "corpse position", r.pos));
        }
        let s = r.take(12);
        let items = r.item_list(t, 21, "corpse items")?;
        out.push(Corpse {
            unk: u32_at(s, 0),
            x: u32_at(s, 4),
            y: u32_at(s, 8),
            items,
        });
    }
    Ok(out)
}

// ----------------------------------------------------------------- write

fn put_list(out: &mut Vec<u8>, items: &[ItemEntry]) -> Result<(), WriteError> {
    let count = u16::try_from(items.len()).map_err(|_| WriteError::Model("item count"))?;
    out.extend_from_slice(&ITEMS_MAGIC.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for it in items {
        out.extend_from_slice(&it.bytes);
    }
    Ok(())
}

/// The model fields the loader would not frame (a save D2RUST writes must
/// load): the skills byte count must be the one the header names (the
/// loader reads +0x2A bytes), the corpse list has at most one entry (two or
/// more is error 21), `kf` holds its item exactly when g ≠ 0 and `jf`
/// holds a list exactly when the loader restores the header's hireling (a
/// list the loader does not read makes the next marker check fail with
/// 22 / 23).
fn check_body(h: &Header, b: &Body, t: &dyn SaveTables) -> Result<(), WriteError> {
    if b.skills.len() != usize::from(h.skill_count) {
        return Err(WriteError::Model("skills length differs from header +0x2A"));
    }
    if b.corpses.len() > 1 {
        return Err(WriteError::Model(
            "more than one corpse (the loader rejects 2+)",
        ));
    }
    if let Some(g) = &b.golem {
        if (g.flag != 0) != g.item.is_some() {
            return Err(WriteError::Model("golem flag and item disagree"));
        }
    }
    if let Some(list) = &b.hireling_items {
        if list.is_some() != t.hireling_restored(&h.hireling) {
            return Err(WriteError::Model(
                "jf item list present without a restored hireling, or missing for one",
            ));
        }
    }
    Ok(())
}

/// Writes the file: sections in file order, then the size at +0x08 and the
/// checksum at +0x0C. The header's `file_size` and
/// `checksum` fields are ignored.
pub fn write(save: &D2s, t: &dyn SaveTables) -> Result<Vec<u8>, WriteError> {
    let h = &save.header;
    let mut out = h.to_bytes().to_vec();
    if let Some(b) = &save.body {
        check_body(h, b, t)?;
        // Quests.
        out.extend_from_slice(&QUEST_MAGIC.to_le_bytes());
        out.extend_from_slice(&QUEST_VERSION.to_le_bytes());
        out.extend_from_slice(&b.quests.size.to_le_bytes());
        for rec in &b.quests.records {
            out.extend_from_slice(rec);
        }
        // Waypoints.
        out.extend_from_slice(&WAYPOINT_MAGIC.to_le_bytes());
        out.extend_from_slice(&b.waypoints.version.to_le_bytes());
        out.extend_from_slice(&b.waypoints.size.to_le_bytes());
        for d in 0..3 {
            out.extend_from_slice(&b.waypoints.records[d]);
            out.extend_from_slice(&b.waypoints.pads[d]);
        }
        // NPC flags.
        out.extend_from_slice(&NPC_MAGIC.to_le_bytes());
        out.extend_from_slice(&b.npcs.size.to_le_bytes());
        for f in b.npcs.a.iter().chain(b.npcs.b.iter()) {
            out.extend_from_slice(f);
        }
        // Stats.
        match &b.stats {
            Stats::Bits(e) => out.extend_from_slice(&write_stats(e, t)),
            Stats::Mask { mask, values } => {
                out.extend_from_slice(&STATS_MAGIC.to_le_bytes());
                out.extend_from_slice(mask);
                for &(_, v) in values {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        // Skills (classes above 6 have no skill list).
        if h.class > 6 {
            return Err(WriteError::Class(h.class));
        }
        out.extend_from_slice(&SKILLS_MAGIC.to_le_bytes());
        out.extend_from_slice(&b.skills);
        // Player items.
        put_list(&mut out, &b.items)?;
        // Corpses.
        let n = u16::try_from(b.corpses.len()).map_err(|_| WriteError::Model("corpse count"))?;
        out.extend_from_slice(&ITEMS_MAGIC.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        for c in &b.corpses {
            out.extend_from_slice(&c.unk.to_le_bytes());
            out.extend_from_slice(&c.x.to_le_bytes());
            out.extend_from_slice(&c.y.to_le_bytes());
            put_list(&mut out, &c.items)?;
        }
        // Hireling items.
        if let Some(list) = &b.hireling_items {
            out.extend_from_slice(&HIRELING_MAGIC.to_le_bytes());
            if let Some(items) = list {
                put_list(&mut out, items)?;
            }
        }
        // Iron Golem.
        if let Some(g) = &b.golem {
            out.extend_from_slice(&GOLEM_MAGIC.to_le_bytes());
            out.push(g.flag);
            if let Some(it) = &g.item {
                out.extend_from_slice(&it.bytes);
            }
        }
        out.extend_from_slice(&b.trailing);
    }
    if out.len() > MAX_FILE {
        return Err(WriteError::Overflow(out.len()));
    }
    finish(&mut out);
    Ok(out)
}

/// Stores the length at +0x08 and the checksum at +0x0C (the checksum is
/// computed with its own field zeroed).
pub fn finish(out: &mut [u8]) {
    let len = out.len() as u32;
    out[0x08..0x0C].copy_from_slice(&len.to_le_bytes());
    out[0x0C..0x10].fill(0);
    let s = checksum(out);
    out[0x0C..0x10].copy_from_slice(&s.to_le_bytes());
}

impl D2s {
    /// The new-character stub the creation screen writes: `flags`
    /// are the creation status flags (bit 0 is added, and 0x20 for class
    /// 5 or 6). `None` when the name does not fit.
    pub fn new_stub(name: &[u8], class: u8, flags: u16, time: u32) -> Option<D2s> {
        let mut h = Header {
            components: STUB_COMPONENTS,
            create_time: time,
            save_time: time,
            unk34: 0,
            hotkeys: [Slot::default(); 16],
            ..Header::default()
        };
        h.set_name(name)?;
        h.class = class;
        h.status = flags
            | status::NEW
            | if matches!(class, 5 | 6) {
                status::EXPANSION
            } else {
                0
            };
        Some(D2s {
            header: h,
            body: None,
        })
    }
}
