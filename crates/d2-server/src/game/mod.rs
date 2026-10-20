//! Headless wired game construction, live data and character restoration.
use std::sync::Arc;

use crate::adapters::character::LoadContext;
use crate::adapters::handlers::skills::wired::WiredSkills;
use crate::adapters::handlers::world::{
    preview_cube_parts, preview_inv_parts, ActionEvents, ActionWorld, Outbox, QuestEnter,
    WiredWorld, WorldHost,
};
use crate::adapters::session::{
    initial_portal_flags, load_new_character_with_items, load_save, GameSetup,
};
use crate::adapters::session_flow::{
    create_flags, CharacterLoader, CreateGame, Loaded, SessionFlow,
};
use crate::adapters::{PlayerData, PlayerFields, SimGame};
use crate::seams::{ClientId, PlayerGate};
use crate::world_data::game::GameTables;
use crate::world_data::tables::{class_picks, drop_tables, hireling_tables, LevelTables, SaveData};
use crate::world_data::{self, WorldFiles};
use d2_data::bin::TableFiles;
use d2_data::tables::{
    decode_all, Charstats, Difficultylevels, Itemstatcost, Levels, Monstats, Objects, Shrines,
    Skills,
};
use d2_formats::animdata::AnimData;
use d2_formats::d2s::{self, D2s, ReadOptions};
use d2_formats::mpq::ArchiveSet;
use d2_native::source::NativeAsset;
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::drlg::maze::Maze;
use d2_sim::drlg::{Drlg, DrlgData, DrlgError, Dungeon, LevelTypes, TileSource};
use d2_sim::game::Game;
use d2_sim::items::inventory::tables::InvTables;
use d2_sim::items::ItemTables;
use d2_sim::monsters::init::GameInfo;
use d2_sim::rng::Seed;
use d2_sim::rules::Ruleset;
use d2_sim::skills::SkillTables;
use d2_sim::stats::StatData;
use d2_sim::units::hooks::UnitData;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionTables, DrlgWorld};
use d2_sim::wiring::economy::{DeathDrops, DropTables, GameFields};
use d2_sim::wiring::worldgen::levels::{SharedTypes, WorldTypes};
use d2_sim::wiring::worldgen::{CreationTables, WorldSim, WorldState, WorldTables};
use d2_sim::world::cube::CubeData;
use d2_sim::world::hirelings::HirelingTables;
use d2_sim::world::npc::HireRow;
use d2_sim::world::objects::ObjectTables;
use d2_sim::world::quests::{PlayerQuests, QuestFlags, QuestTables};
use d2_sim::world::vendors::VendorTables;
use d2_sim::world::waypoints::WaypointData;

use d2_sim::drlg::outdoor::{SubFile, SubFiles};
use d2_sim::drlg::preset::{Ds1Input, Ds1Source};

use crate::file_source::{CanonicalPath, FileSource};
use crate::game_files::GameFiles;
use crate::game_thread::ThreadStopped;
use crate::local_seams::*;
use crate::rest::AppRest;

pub mod appearance;
mod builder;
pub mod character_save;
pub mod client_tables;
pub mod composite_names;
mod live_data;
pub mod rows;
pub mod save_full;
pub mod save_gaps;
mod save_load;
pub use builder::*;
use client_tables::fixed_objects;
pub use live_data::*;
pub use save_load::*;
pub type Sim = SimGame<WorldSim<LocalSeams>, World>;
pub type World = WiredWorld<AppRest, WiredSkills>;

impl Sim {
    /// Closes the client's open interaction (vendor trade or store, stash,
    /// cube, player trade) before a re-entry replays its state (contracts
    /// v12 3.7): the player's interact info and the NPC's or partner's side
    /// are reset, and the S→C messages the client expects for the close are
    /// returned. Another player's trade close is queued to that player.
    pub fn close_client_interactions(&mut self, client: ClientId) -> Vec<Vec<u8>> {
        let Some(player) = self.player_of(client) else {
            return Vec::new();
        };
        self.world
            .close_client_interactions(&mut self.game, &mut self.events, player)
    }

    /// Sends the client the live base skill levels (S→C 0x94) of its
    /// player. A join of a loaded character sends them; a new character's
    /// join does not, so after play a re-entry's journal has no 0x94 and
    /// the client's skill list would stay empty (contracts v12 3.7: the
    /// resend covers what the client clears).
    pub fn reenter_character_state(&mut self, client: ClientId) {
        use d2_sim::wiring::action::Pending;
        let Some(player) = self.player_of(client) else {
            return;
        };
        let a = self.events.action();
        let Some(guid) = a.sys.units.get(player).map(|r| r.guid) else {
            return;
        };
        let levels = a
            .sys
            .hooks
            .skill_lists
            .get(&player)
            .map(crate::adapters::session::join_base_levels)
            .unwrap_or_default();
        if let Some(m) = d2_sim::units::messages::base_skill_levels(guid, &levels) {
            a.sys.hooks.x.send(player, &m);
        }
    }
}

/// The Rogue Encampment (act 0's town, where game entry places the
/// player), Cold Plains (act 0) and
/// Lut Gholein (act 1).
pub const ACT1_TOWN: u32 = 1;
/// The Blood Moor (act 0), east of the synthetic town's room.
pub const BLOOD_MOOR: u32 = 2;
pub const COLD_PLAINS: u32 = 3;
/// Stony Field: a waypoint level not built at game creation (its level
/// init runs on arrival).
pub const STONY_FIELD: u32 = 4;
/// The Den of Evil (act 0): the cave entrance in the Blood Moor is a
/// level warp (its `lvlwarp` id is the `levels` row's `Warp` of the Den slot).
pub const DEN_OF_EVIL: u32 = 8;
pub const ACT2_TOWN: u32 = 40;
/// Harrogath (act 4, the fifth act; `levels` row 109).
pub const ACT5_TOWN: u32 = 109;
/// Kurast Docks (act 2's town, `levels` row 75) and the Pandemonium
/// Fortress (act 3's, row 103).
pub const KURAST_DOCKS: u32 = 75;
/// The stash object's class (`objects.txt` row 267,
/// `BANK_CLASS`).
pub const STASH_CLASS: u32 = 267;
pub const PANDEMONIUM_FORTRESS: u32 = 103;
/// Catacombs Level 4, Andariel's lair (act 0; a flat level in the
/// synthetic world, reached by a level warp: D2RUST's own, unverified).
pub const CATACOMBS_4: u32 = 37;
/// The default game seed.
pub const DEFAULT_SEED: u32 = 1234;

/// Choose the game's DRLG seed: an explicit fixed seed wins, then a loaded
/// save's map seed when its difficulty town byte has bit 0x80, then `drawn`.
/// Dev paths use [`DEFAULT_SEED`] for reproducibility; the front end supplies
/// [`clock_map_seed`].
pub fn game_seed_or(character: &Character, fixed: Option<u32>, drawn: u32) -> u32 {
    if let Some(n) = fixed {
        return n;
    }
    match character {
        Character::Save(save, ctx) => {
            let t = save.header.towns[usize::from(ctx.difficulty).min(2)];
            if t & 0x80 != 0 {
                save.header.map_seed
            } else {
                drawn
            }
        }
        _ => drawn,
    }
}

/// [`game_seed_or`] with [`DEFAULT_SEED`] for the drawn seed: the dev paths.
pub fn game_seed(character: &Character, fixed: Option<u32>) -> u32 {
    game_seed_or(character, fixed, DEFAULT_SEED)
}

/// Draw a map seed without the fixed-seed switch.
/// Seed with `init_low(time_value(clock + qpc))`, step once, then return
/// `time_value(clock + lo)`. The caller reads both clock inputs once.
pub fn drawn_map_seed(clock: u32, qpc: u32) -> u32 {
    let mut game = d2_sim::rng::Seed::init_low(d2_sim::rng::time_value(clock.wrapping_add(qpc)));
    let lo = game.step();
    d2_sim::rng::time_value(clock.wrapping_add(lo))
}

/// [`drawn_map_seed`] from the wall clock: the front end's new game. The
/// client reads the clock; the sim only gets the seed.
pub fn clock_map_seed() -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // time() in seconds plus a millisecond count for GetTickCount; the
    // nanoseconds stand in for the performance counter.
    let clock = (now.as_secs() as u32).wrapping_add(now.as_millis() as u32);
    drawn_map_seed(clock, now.subsec_nanos())
}
/// Single-player game type carried by the client's create message: 3.
pub const GAME_TYPE: u8 = 3;
/// Duriel's Lair (`levels` row 73, act 1).
pub const DURIELS_LAIR: u32 = 73;
/// Durance of Hate levels 1 and 2.
pub const DURANCE_1: u32 = 100;
pub const DURANCE_2: u32 = 101;
/// Duriel's `monstats` row.
pub const DURIEL_CLASS: u32 = 211;

/// The player's character class (1, sorceress, as in the server tests).
pub const PLAYER_CLASS: u32 = 1;
/// The character's name (0x59 bytes 6..22, zero-padded).
pub const PLAYER_NAME: &[u8] = b"Sorceress";

/// Normal expansion game settings sent in S-to-C 0x01. The arena flags
/// carry the expansion bit and bit 2; the arena itself is not modeled.
pub const GAME_SETUP: GameSetup = GameSetup {
    difficulty: 0,
    arena_flags: 0x0010_0004,
    expansion: true,
    ladder: false,
};

/// Expansion character flags for C-to-S 0x67: expansion bit plus bit 2.
pub const CREATE_FLAGS_EXPANSION: u32 = create_flags::EXPANSION | 0x4;

/// The 0x67 u32@0x27 of a classic character.
///
/// Bit 2 alone, without the expansion bit 20.
pub const CREATE_FLAGS_CLASSIC: u32 = 0x4;

/// The local client's C→S 0x67 for `character`: game name empty (byte 1 = 0),
/// game type 3 (client type 0), the character's class and name
/// ([`Character::Save`]: the save's name ; class byte 0), template 0, the
/// game's difficulty, u16@0x25 = 0, the flags of an expansion or a classic
/// character (save status bit 5), @0x2B = @0x2C = 0, language id 0. Bytes after
/// a name's NUL are zero.
pub fn create_request_for(character: &Character, width: i32, height: i32) -> CreateGame {
    let (class, name, expansion) = match character {
        Character::New => (PLAYER_CLASS as u8, PLAYER_NAME, GAME_SETUP.expansion),
        Character::Named(c) => (c.class, c.name(), GAME_SETUP.expansion),
        // A saved-character join sends class byte zero. The server loads the
        // actual class from the save header (formats/d2s.md).
        Character::Save(save, _) => (
            0,
            save.header.name_bytes(),
            save.header.status & d2_formats::d2s::status::EXPANSION != 0,
        ),
        Character::Refused(r) => (
            0,
            &r.name[..r.name.iter().position(|&b| b == 0).unwrap_or(16)],
            r.expansion,
        ),
    };
    let mut char_name = [0u8; 16];
    char_name[..name.len()].copy_from_slice(name);
    CreateGame {
        game_type: GAME_TYPE,
        class,
        template: 0,
        difficulty: character.difficulty(),
        char_name,
        arena: 0,
        flags: if expansion {
            CREATE_FLAGS_EXPANSION
        } else {
            CREATE_FLAGS_CLASSIC
        },
        view_gap: wire_view_gap(width, height),
        unk: 0,
        locale: 0,
        ..CreateGame::default()
    }
}

/// The CreateGame view gap of a `width` x `height` frame: 0 (the near
/// gap, a 4:3 game) for the fixed 800x600 frame, else the wider view's
/// gap ([`d2_sim::drlg::view_gap_for_frame`]), which the server gives the
/// game's DRLGs.
pub fn wire_view_gap(width: i32, height: i32) -> u8 {
    let gap = d2_sim::drlg::view_gap_for_frame(width, height);
    if gap == d2_sim::drlg::NEAR_GAP {
        0
    } else {
        gap.clamp(0, i32::from(u8::MAX)) as u8
    }
}

/// The character supplied to the session flow's join loader.
#[derive(Debug, Clone, Default)]
pub enum Character {
    /// A new character of the 0x67 request's class and name, as the save loader
    /// leaves a player (no room, at (0, 0), mode 1), knowing only the town
    /// waypoint (bit 0, B034). It is the stub load
    /// (`crate::adapters::session::load_new_character`), so the join sends
    /// 0x5F and the two 0x23.
    #[default]
    New,
    /// [`Character::New`] with the class and name of `d2-client play
    /// --new <class> <name>` (a CLI stand-in
    /// for the select / create screens). Held in
    /// memory only: nothing is written to disk.
    Named(NewCharacter),
    /// A parsed `.d2s` loaded onto the new player
    /// (`crate::adapters::session::load_save`); `d2-client play
    /// --save` reads one with [`LiveData::read_save`].
    Save(Box<D2s>, LoadContext),
    /// A save the load refuses: the join reports
    /// the result in S→C 0xB4 and the client is removed.
    /// Only the request's name and flags are kept.
    Refused(RefusedLoad),
}

/// A refused load: the result code the join reports and what the 0x67
/// request carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedLoad {
    /// The load result of the save loader (the 0xB4 payload).
    pub result: u32,
    /// The save's character name, NUL-padded.
    pub name: [u8; 16],
    /// The save's expansion bit (status bit 5), for the request's flags.
    pub expansion: bool,
    /// The game's difficulty.
    pub difficulty: u8,
    /// The save's class, used for the player unit when the load
    /// fails after the unit exists.
    pub class: u8,
    /// The failure is in a section reader (internal codes 15–23): the
    /// player unit exists and its add messages were queued.
    pub after_player: bool,
    /// The skill levels the load had applied: set when a section
    /// after the skills failed (internal codes 20–23), so the join sends S→C
    /// 0x94 before the 0xB4.
    pub skills: Option<Vec<u8>>,
}

impl Character {
    /// The difficulty the game runs on: a save's load context, else a
    /// named character's own (Normal for [`Character::New`]).
    pub fn difficulty(&self) -> u8 {
        match self {
            Character::New => GAME_SETUP.difficulty,
            Character::Named(c) => c.difficulty,
            Character::Save(_, ctx) => ctx.difficulty,
            Character::Refused(r) => r.difficulty,
        }
    }

    /// This character on difficulty `d` (`play --difficulty`). A save's
    /// difficulty is set at load ([`load_character`]); a default new
    /// character becomes the stand-in sorceress as a named one.
    pub fn with_difficulty(self, d: u8) -> Character {
        match self {
            Character::New => {
                let mut name = [0u8; 16];
                name[..PLAYER_NAME.len()].copy_from_slice(PLAYER_NAME);
                Character::Named(NewCharacter {
                    class: PLAYER_CLASS as u8,
                    name,
                    difficulty: d,
                })
            }
            Character::Named(c) => Character::Named(NewCharacter { difficulty: d, ..c }),
            Character::Save(s, mut ctx) => {
                ctx.difficulty = d;
                Character::Save(s, ctx)
            }
            Character::Refused(r) => Character::Refused(RefusedLoad { difficulty: d, ..r }),
        }
    }
}

/// Parses `--difficulty`: `normal`, `nightmare`, `hell` (any case) or 0–2.
pub fn parse_difficulty(s: &str) -> Option<u8> {
    match s.to_ascii_lowercase().as_str() {
        "normal" | "0" => Some(0),
        "nightmare" | "1" => Some(1),
        "hell" | "2" => Some(2),
        _ => None,
    }
}

/// The class names of `play --new` in class-id order (`charstats` rows
/// 0–6; the inventory class ids are the same).
pub const CLASS_NAMES: [&str; 7] = [
    "amazon",
    "sorceress",
    "necromancer",
    "paladin",
    "barbarian",
    "druid",
    "assassin",
];

/// Longest character name: a NUL must fit within the request's 16-byte field.
pub const MAX_NAME_LEN: usize = 15;

/// The class and name of a new character (`play --new`, decision D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewCharacter {
    /// Class id 0–6 (the 0x67 class u8@0x12; ≥ 7 is refused).
    pub class: u8,
    /// The name, NUL-padded (bytes after the name are 0).
    pub name: [u8; 16],
    /// The game's difficulty 0–2 (`play --difficulty`; Normal by default).
    pub difficulty: u8,
}

impl NewCharacter {
    /// The name bytes up to the first NUL.
    pub fn name(&self) -> &[u8] {
        let n = self.name.iter().position(|&b| b == 0).unwrap_or(16);
        &self.name[..n]
    }
}

/// Why `play --new <class> <name>` was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NewCharacterError {
    #[error("unknown class {0:?}: use 0-6 or one of {names}", names = CLASS_NAMES.join(", "))]
    Class(String),
    #[error("character name {0:?}: {1}")]
    Name(String, &'static str),
}

/// The class id of `s`: a number 0–6 or a [`CLASS_NAMES`] entry (any
/// case).
pub fn parse_class(s: &str) -> Result<u8, NewCharacterError> {
    if let Ok(n) = s.parse::<u8>() {
        return if n < 7 {
            Ok(n)
        } else {
            Err(NewCharacterError::Class(s.to_owned()))
        };
    }
    CLASS_NAMES
        .iter()
        .position(|c| c.eq_ignore_ascii_case(s))
        .map(|i| i as u8)
        .ok_or_else(|| NewCharacterError::Class(s.to_owned()))
}

/// The character of `play --new <class> <name>` (decision D3): class by
/// name or 0–6; the name 1–[`MAX_NAME_LEN`] bytes so the 0x67 check
/// passes.
pub fn new_character(class: &str, name: &str) -> Result<Character, NewCharacterError> {
    let class = parse_class(class)?;
    let bad = |why| NewCharacterError::Name(name.to_owned(), why);
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err(bad("must be 1 to 15 bytes"));
    }
    // D2RUST's own, unverified: the create screen's name rules are not
    // specified (no menu spec); ASCII letters, digits, `-` and `_` keep
    // the name printable in every font and safe in a file name.
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(bad("only ASCII letters, digits, '-' and '_'"));
    }
    let mut bytes = [0u8; 16];
    bytes[..name.len()].copy_from_slice(name.as_bytes());
    Ok(Character::Named(NewCharacter {
        class,
        name: bytes,
        difficulty: 0,
    }))
}

/// D2RUST diagnostic when player allocation fails. Save-load failures use
/// the format loader's result instead. The flow records a load refusal
/// and removes the staged character.
pub const LOAD_FAILED: u32 = u32::MAX;

/// Errors building the game.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("DRLG: {0:?}")]
    Drlg(DrlgError),
    #[error("{0}")]
    Setup(String),
    #[error("game tables: {0}")]
    Tables(String),
    #[error("level data: {0}")]
    World(#[from] crate::world_data::WorldDataError),
    #[error("archives in {dir}: {message}")]
    Archives { dir: String, message: String },
    /// No game directory given: the game plays only on the user's own
    /// files (`$D2_GAME_DIR`, or `--native DIR`).
    #[error("no game files: put D2RUST-Game.exe in the game folder (next to d2data.mpq), pass --game-dir <folder>, or set D2_GAME_DIR")]
    NoGameDir,
    #[error("no objects row has operate function 23 and init function 17")]
    NoWaypointObject,
    /// Game creation's NPC or quest control.
    #[error("game creation: {0}")]
    Creation(#[from] d2_sim::wiring::worldgen::CreationError),
    /// `--save`: the file could not be read, or it is not a save the
    /// reader takes.
    #[error("save {path}: {message}")]
    Save { path: String, message: String },
    #[error(transparent)]
    Thread(#[from] ThreadStopped),
}

/// The tables game creation and the wired host read beyond the DRLG's.
struct GameParts {
    action: ActionTables,
    stats: StatData,
    units: UnitData,
    world: WorldTables,
    objects: ObjectTables,
    monstats: Vec<Monstats>,
    hire_rows: Vec<HireRow>,
    items: ItemTables,
    vendors: VendorTables,
    anim: Option<Arc<AnimData>>,
    monster_sequences: Option<Arc<d2_sim::skills::sequences::MonsterSequences>>,
    vitals: Option<Arc<VitalsTables>>,
    /// The skill bodies' table data (`ActionHooks::bodies`: pet types,
    /// state groups); `None`: synthetic.
    bodies: Option<Arc<d2_sim::skills::use_::bodies::BodyTables>>,
    /// The chest drop's tables; `None`: no drop (synthetic).
    drops: Option<Arc<DropTables>>,
    /// The drop helpers' class picks (`DeathDrops::with_picks`).
    picks: Option<Arc<d2_sim::treasure::class_pick::ClassPicks>>,
    /// `None`: the mercenary calls report no tables (synthetic).
    hirelings: Option<HirelingTables>,
    /// The inventory tables of the wired host's inventory model (the new
    /// character's start items); none for
    /// synthetic data (the start items then stay unapplied).
    inventory: Option<InvTables>,
    /// The Horadric Cube's recipes and item columns; none for synthetic
    /// data (the cube then stays a stub).
    cube: Option<CubeData>,
}

impl GameParts {
    /// Hands a mod game's rules to the item tables, the game's own and
    /// the drops' (`docs/plans/mods/seams.md` MS7): item creation and
    /// properties read a mod game's item overrides from there. Under
    /// [`Ruleset::Original`] the tables keep D2RUST's rules and the drop
    /// tables stay shared with [`LiveData`].
    fn set_rules(&mut self, rules: &Ruleset) {
        if rules.is_original() {
            return;
        }
        self.items.rules = rules.clone();
        if let Some(drops) = self.drops.as_mut() {
            let mut own = DropTables::clone(drops);
            own.items.rules = rules.clone();
            *drops = Arc::new(own);
        }
    }

    /// The user's tables (`GameTables`), the drop and hireling tables of
    /// [`LiveData`], the world state's level types over the live data.
    fn live(d: &LiveData) -> Result<Self, BuildError> {
        let t = &d.tables;
        Ok(GameParts {
            action: t.action_tables()?,
            stats: t.stat_data()?,
            units: t.unit_data(GAME_SETUP.expansion)?,
            world: t.world_tables()?,
            objects: t.object_tables()?,
            monstats: t.rows()?,
            hire_rows: t.hire_rows()?,
            items: t.item_tables()?,
            vendors: t.vendor_tables()?,
            anim: Some(Arc::new(t.anim.clone())),
            monster_sequences: Some(Arc::new(t.monster_sequences()?)),
            vitals: Some(Arc::new(t.vitals()?)),
            bodies: Some(Arc::new(t.body_tables()?)),
            drops: Some(d.drops.clone()),
            picks: Some(d.picks.clone()),
            hirelings: Some(d.hirelings.clone()),
            inventory: Some(
                InvTables::from_fixed(&t.fixed)
                    .map_err(|e| BuildError::Tables(format!("inventory tables: {e}")))?,
            ),
            cube: Some(
                CubeData::from_fixed(&t.fixed)
                    .map_err(|e| BuildError::Tables(format!("cube tables: {e}")))?,
            ),
        })
    }
}

/// Construct the wired simulation with its rules and appearance provider set before any unit runs.
pub(super) fn build_game(
    data: &GameData,
    seed: u32,
    character: Character,
    rules: Ruleset,
    appearance: Option<Arc<dyn crate::local_seams::UnitAppearance>>,
) -> Result<Sim, BuildError> {
    let GameData::Live(d) = data;
    let wp_tables = data.tables();
    let (mut levels, mut parts) = (LevelSource::live(d, seed), GameParts::live(d)?);
    parts.set_rules(&rules);
    let shared_types = levels.shared.clone();
    let mut dungeon = Dungeon::default();
    for (act, init_seed, town) in levels.acts.iter().copied() {
        dungeon.acts[usize::from(act)] = Some(
            Drlg::create(
                act,
                init_seed,
                0,
                town,
                false,
                &levels.data,
                levels.types.as_mut(),
            )
            .map_err(BuildError::Drlg)?,
        );
    }
    let world = DrlgWorld {
        dungeon,
        data: levels.data,
        tiles: levels.tiles,
        types: levels.types,
    };
    // The fixed-seed branch (`--seed N`): the game seed
    // is `{N, 666}`, unstepped.
    let seams = LocalSeams {
        monsters: crate::monster_ai::MonsterAi::from_tables(
            &parts.action.combat.monstats,
            &parts.action.combat.monstats2,
        ),
        ..LocalSeams::default()
    };
    let mut hooks = ActionHooks::new(Arc::new(parts.action), world, Seed::init_low(seed), seams);
    hooks.rules = rules.clone();
    hooks.anim_data = parts.anim;
    hooks.monster_sequences = parts.monster_sequences;
    // The server's animation names follow the client art's name rules.
    hooks.x.looks = appearance;
    // Init function 17 of the waypoint objects (`waypoints.md` initialization) on
    // the game's tables.
    let wp_objects = fixed_objects(d);
    hooks.waypoint_init = Some(Arc::new(WaypointData::new(&wp_tables.levels, &wp_objects)));
    hooks.vitals = parts.vitals;
    hooks.bodies = parts.bodies;
    // The hireling calls (save restore; join follow; act change) run on the
    // wired host, which holds the hireling lists when the game has
    // `hireling.txt`; without the queue a saved hireling is never restored.
    if parts.hirelings.is_some() {
        hooks.hireling_calls = Some(Vec::new());
    }
    // The client vitals sync: life, mana,
    // stamina and position sent to the client at the end of each tick.
    hooks.enable_vitals_sync();
    // Allocate the corpse at the player's dead-mode start, before its
    // announcement messages are sent (vitals.md section 4.7).
    hooks.death.allocate_corpses = true;
    // Game entry places through the path provider; on before any unit is
    // allocated.
    hooks
        .enable_paths()
        .map_err(|e| BuildError::Setup(format!("path tables: {e:?}")))?;
    // Enable registered item and skill event handlers (combat/events.md).
    hooks.enable_unit_events();
    // The floor drop's walk-back field: monster
    // and chest drops search their spot with it.
    if let Some(paths) = hooks.paths.as_mut() {
        paths.field = Some(d.expfield.clone());
    }
    // The inactive store: a room the tick frees
    // keeps its units' records, and its next build restores them.
    hooks.enable_inactive_store();
    let info = GameInfo {
        expansion: GAME_SETUP.expansion,
        difficulty: character.difficulty(),
        game_type: GAME_TYPE,
        ladder: GAME_SETUP.ladder,
        ..GameInfo::default()
    };
    // One level-type state for the DRLG and population (`SharedTypes`):
    // the presets of a generated room are the ones population places
    // (q-fixture-migrate: a second, fresh state left every town without
    // its preset NPCs and objects on the user's files).
    let state = WorldState::new(shared_types, Arc::new(parts.world), info);
    let mut sim = WorldSim::new(Arc::new(parts.stats), parts.units, hooks, state);
    // Game creation: the creation fields to their home,
    // then the four seeded controls in order, before any unit.
    let fields = GameFields {
        difficulty: character.difficulty(),
        game_type: GAME_TYPE,
        ladder: GAME_SETUP.ladder,
        ..GameFields::new(Seed::init_low(seed), GAME_SETUP.expansion)
    };
    ActionEvents::create_game(&mut sim, &fields);
    let quest_tables =
        QuestTables::load().map_err(|e| BuildError::Tables(format!("quest tables: {e}")))?;
    let created = sim.create_game(CreationTables {
        objects: Arc::new(parts.objects),
        monstats: &parts.monstats,
        hirelings: parts.hire_rows,
        quests: &quest_tables,
    })?;
    // The chest drop's state: its seed, creation
    // fields and unique bits are the action wiring's.
    let picks = parts.picks;
    sim.action.hooks().object_drops = parts.drops.map(|t| {
        let d = DeathDrops::new(t, GameFields::new(Seed::init_low(0), false));
        Box::new(match picks {
            Some(p) => d.with_picks(p),
            None => d,
        })
    });
    let mut game = Game::new();
    game.tick_rate = d2_sim::tick::TickRate::GAME;
    let start_levels = [(0u8, ACT1_TOWN), (0, COLD_PLAINS), (1, ACT2_TOWN)];
    for (act, level) in start_levels {
        game.lists
            .ensure_act(act)
            .map_err(|e| BuildError::Setup(format!("act {act}: {e:?}")))?;
        let r = sim
            .action
            .hooks()
            .drlg
            .with_act(act, &mut game.lists, |d, svc| {
                let l = d.get_or_alloc_level(svc.data, svc.types, level)?;
                // A town is generated at act creation; any other level here
                // once.
                if d.level_rooms(l).is_empty() {
                    d.generate_level(svc.data, svc.types, l)?;
                }
                let Some(&r) = d.level_rooms(l).first() else {
                    return Ok(None);
                };
                let rect = d.room(r).rect;
                Ok(d.stream_room(svc, r)?.map(|id| (id, rect)))
            })
            .ok_or_else(|| BuildError::Setup(format!("act {act} has no DRLG")))?
            .map_err(BuildError::Drlg)?
            .ok_or_else(|| BuildError::Setup(format!("level {level}: no room streamed")))?;
        let _ = r;
    }
    // The Act II DRLG chose its staff tomb at creation.
    if let Some(t) = sim
        .action
        .hooks()
        .drlg
        .with_act(1, &mut game.lists, |d, _| d.staff_tomb)
    {
        sim.action.hooks().x.staff_tomb = t;
    }
    let interact_classes: Vec<u16> = parts
        .monstats
        .iter()
        .enumerate()
        .filter(|(_, m)| m.npc && m.interact)
        .map(|(i, _)| i as u16)
        .collect();
    // The wired host on the created controls.
    let action = ActionWorld {
        waypoints: Some(WaypointData::new(&wp_tables.levels, &wp_objects)),
        // The skill handlers (C→S 0x05–0x11, 0x3A–0x3C) on the action
        // wiring, their open seams on `LocalSeams` (`super::skill_rest`).
        skills: WiredSkills::default(),
        ..ActionWorld::default()
    };
    let rest = AppRest {
        expansion: GAME_SETUP.expansion,
        snap: sim.action.sys.hooks.x.snap.clone(),
        ..AppRest::default()
    };
    // `WiredWorld::now` (store generation and refresh) is the host clock in
    // ms, set by each host frame
    // (`host_tick`); 0 until the first frame.
    let mut world = WiredWorld::new(
        action,
        parts.items,
        created.quests,
        created.npc,
        parts.vendors,
        rest,
        0,
    );
    world.state.hireling_tables = parts.hirelings;
    // Monster init does not embed the interaction lists yet: the host
    // registers the `interact` NPC units (D2RUST's own, unverified).
    world.interact_classes = interact_classes;
    // The play host's inventory model (`play-server` seam, D1 preview
    // fills in `PreviewMoveRest`): the new character's start items.
    world.inventory = parts.inventory.map(preview_inv_parts);
    // The cube (D2RUST's own, unverified): the user's `cubemain`.
    world.cube = parts.cube.map(preview_cube_parts);
    // New characters receive only their charstats start-item slots.
    let mut s: Sim = SimGame::with_world(game, sim, world);
    // A mod game's rules also take its mod messages (MS13).
    s.rules = rules;
    s.announce_ground = true;
    s.events.action.hooks().item_marks = true;
    s.events.action.hooks().arena = Some(Default::default());
    s.set_host_sync(sync_seams);
    s.set_world_sync(world_sync);
    // The session sequence runs on the client's
    // C→S 0x67 / 0x6B: game creation (the client record, 0x01, 0x00,
    // 0x02; state 1), then the join (this loader, the player's add
    // messages, 0x0B, …, 0x03, 0x53, game entry with its room switch;
    // state 3). The next tick populates the town's rooms, the client's room
    // is ready and the client pass sends 0x04.
    s.set_session(SessionFlow::new(loader(one_character(character))));
    Ok(s)
}

/// Run join quest entry and queue its messages. Errors are logged and
/// the join continues.
fn quest_entry(s: &mut Sim, player: UnitId, new_character: bool) {
    let modes: &[u8] = if new_character { &[1, 0] } else { &[0] };
    for &mode in modes {
        let r = WorldHost::quests(
            &mut s.world,
            &mut s.game,
            &mut s.events,
            QuestEnter { player, mode },
        );
        if let Some(Err(e)) = r {
            s.events
                .action
                .hooks()
                .x
                .log
                .push(format!("join: quest entry mode {mode}: {e}"));
        }
    }
    let sent: Vec<Vec<u8>> = s
        .world
        .rest
        .take_sent()
        .into_iter()
        .filter(|(u, _)| *u == player)
        .map(|(_, b)| b)
        .collect();
    if !sent.is_empty() {
        s.events
            .action
            .sys
            .hooks
            .session
            .join_quest
            .insert(player, sent);
    }
}
