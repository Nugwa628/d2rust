//! NPC interaction: NPC records and the NPC-control seed, talk and chat,
//! menu actions, healing, Cain identify, mercenaries ([`hire`]), NPC
//! services ([`services`]) and the S→C 0x2A / 0x58 / 0x4E / 0x9B
//! messages. A mod game's NPC roles and menu actions come from its
//! [`NpcProvider`].
//!
//! Everything outside this module is reached through two seams:
//! [`NpcWorld`] (game, units, stats, items, quests, messages) and
//! [`NpcVendors`] (store, gamble, prices and repair, implemented by
//! `world::vendors`; the provider wires that module into this
//! trait).

use d2_data::bin::BinTable;
use d2_data::tables::{Hireling, Monstats, Record};

use crate::rng::Seed;
use crate::units::UnitId;
use crate::world::quests::{QuestFlags, TextList};
use crate::world::{tsv_num, tsv_rows, TsvError};

pub mod hire;
pub mod provider;
pub mod services;

#[cfg(test)]
mod tests;

pub use hire::{HireList, HireOffer, HireRow, HireSlot, MercInit};
pub use provider::{NpcHost, NpcProvider, NpcRoles};
pub use services::{ImbueMods, InvEntry, ItemFacts, Place};

/// Monstats classes this module names.
pub mod class {
    pub const CAIN1: u16 = 146;
    pub const GHEED: u16 = 147;
    pub const AKARA: u16 = 148;
    pub const KASHYA: u16 = 150;
    pub const CHARSI: u16 = 154;
    pub const WARRIV1: u16 = 155;
    pub const WARRIV2: u16 = 175;
    pub const ATMA: u16 = 176;
    pub const DROGNAN: u16 = 177;
    pub const FARA: u16 = 178;
    pub const GREIZ: u16 = 198;
    pub const ELZIX: u16 = 199;
    pub const LYSANDER: u16 = 202;
    pub const MESHIF1: u16 = 210;
    pub const CAIN2: u16 = 244;
    pub const CAIN3: u16 = 245;
    pub const CAIN4: u16 = 246;
    pub const ASHEARA: u16 = 252;
    pub const HRATLI: u16 = 253;
    pub const ALKOR: u16 = 254;
    pub const ORMUS: u16 = 255;
    pub const HALBU: u16 = 257;
    pub const MESHIF2: u16 = 264;
    pub const CAIN5: u16 = 265;
    pub const TYRAEL2: u16 = 367;
    pub const JAMELLA: u16 = 405;
    pub const LARZUK: u16 = 511;
    pub const DREHYA: u16 = 512;
    pub const MALAH: u16 = 513;
    pub const NIHLATHAK: u16 = 514;
    pub const QUAL_KEHK: u16 = 515;
    pub const CAIN6: u16 = 520;
}

use class::*;

/// NPCs with a trade action (menu action 1).
pub const TRADERS: [u16; 16] = [
    GHEED, AKARA, CHARSI, DROGNAN, FARA, ELZIX, LYSANDER, ASHEARA, HRATLI, ALKOR, ORMUS, HALBU,
    JAMELLA, LARZUK, DREHYA, MALAH,
];
/// NPCs with a gamble action (menu action 2).
pub const GAMBLERS: [u16; 6] = [GHEED, ELZIX, ALKOR, JAMELLA, DREHYA, NIHLATHAK];
/// Healers.
pub const HEALERS: [u16; 6] = [AKARA, ATMA, FARA, ORMUS, JAMELLA, MALAH];
/// The identify sound event (`cursor_identify_item`).
const SOUND_IDENTIFY: u16 = 6;

/// The Cains that identify (cain1 excluded).
pub const IDENTIFIERS: [u16; 5] = [CAIN2, CAIN3, CAIN4, CAIN5, CAIN6];
/// Mercenary sellers (hire list).
pub const SELLERS: [u16; 4] = [KASHYA, GREIZ, ASHEARA, QUAL_KEHK];
/// NPCs that resurrect.
pub const RESURRECTORS: [u16; 5] = [KASHYA, GREIZ, ASHEARA, TYRAEL2, QUAL_KEHK];

/// Records allocated per game.
pub const RECORD_MAX: usize = 64;
/// The interaction distances.
pub const START_DISTANCE: i32 = 50;
pub const TALK_DISTANCE: i32 = 6;
pub const APPROACH_DISTANCE: i32 = 8;
/// The AI parameter an interaction sets on the NPC.
pub const AI_PARAM: u32 = 0x28;
/// The heal sound attached to the NPC.
pub const SOUND_HEAL: u16 = 10;
/// Unit type of a monster (interact unit type of an NPC).
pub const UNIT_MONSTER: u8 = 1;

/// Stat ids used here.
pub mod stat {
    pub const LIFE: u16 = 6;
    pub const MANA: u16 = 8;
    pub const STAMINA: u16 = 10;
    pub const LEVEL: u16 = 12;
    pub const GOLD: u16 = 14;
    pub const ITEM_194: u16 = 194;
}

/// Node states of an interaction list.
pub mod talk {
    pub const TALKING: u8 = 0;
    pub const CHATTING: u8 = 1;
    pub const TRADING: u8 = 2;
}

/// S→C 0x2A result codes and kinds.
pub mod code {
    pub const BOUGHT: u8 = 0;
    pub const SOLD: u8 = 1;
    pub const REPAIRED: u8 = 2;
    pub const IDENTIFIED: u8 = 3;
    pub const MERC: u8 = 5;
    pub const HEALED: u8 = 6;
    pub const BUY_REFUSED: u8 = 7;
    pub const REFUSED: u8 = 9;
    pub const NO_ROOM: u8 = 10;
    pub const GATE: u8 = 11;
    pub const NO_GOLD: u8 = 12;
    pub const NOTHING_TO_HEAL: u8 = 14;
    pub const NOT_PLACED: u8 = 15;
}

/// Fatal NPC errors, returned instead of aborting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NpcError {
    #[error("more than {RECORD_MAX} interact monstats rows")]
    TooManyRecords,
    #[error("no hireling row for seller {seller}, difficulty column {difficulty}")]
    NoHirelingRow { seller: u16, difficulty: u32 },
    #[error("hireling name range {first}..={last} does not fit the 69 hire slots")]
    HireListSize { first: u16, last: u16 },
    #[error("hireling table: {0}")]
    Table(String),
    #[error("vendors.tsv: {0}")]
    Tsv(#[from] TsvError),
    #[error("vendor: {0}")]
    Vendor(#[from] crate::world::vendors::price::PriceFatal),
}

// ------------------------------------------------------------ NPC table

/// `vendors.tsv` (shared with the vendors module), embedded: the NPC
/// table and every NPC's roles.
pub const VENDORS_TSV: &str = include_str!("../../../../tables/world/vendors.tsv");

const VENDORS_HEADER: &[&str] = &[
    "npc",
    "name",
    "act",
    "trader",
    "force_vendor",
    "cache",
    "refresh_flag",
    "gamble_flag",
    "trade_action",
    "gamble_action",
    "heals",
    "identifies",
    "hire_list",
    "resurrects",
];

/// One NPC table entry with the role columns of `vendors.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcTableRow {
    pub class: u16,
    pub name: String,
    pub act: u8,
    pub trader: u8,
    /// NPC table byte 6.
    pub byte6: u8,
    pub trade_action: bool,
    pub gamble_action: bool,
    pub heals: bool,
    pub identifies: bool,
    pub hire_list: bool,
    pub resurrects: bool,
}

/// Parses `vendors.tsv` strictly.
pub fn parse_npc_table(text: &str) -> Result<Vec<NpcTableRow>, TsvError> {
    const T: &str = "vendors.tsv";
    let mut out = Vec::new();
    for (line, c) in tsv_rows(T, text, VENDORS_HEADER)? {
        let num = |i: usize, max: u32| -> Result<u32, TsvError> {
            let v = tsv_num(T, line, VENDORS_HEADER[i], c[i])?;
            if v > max {
                return Err(TsvError::Value {
                    table: T,
                    line,
                    column: VENDORS_HEADER[i],
                    value: c[i].to_string(),
                });
            }
            Ok(v)
        };
        out.push(NpcTableRow {
            class: num(0, 0xFFFF)? as u16,
            name: c[1].to_string(),
            act: num(2, 4)? as u8,
            trader: num(3, 1)? as u8,
            byte6: num(4, 1)? as u8,
            trade_action: num(8, 1)? == 1,
            gamble_action: num(9, 1)? == 1,
            heals: num(10, 1)? == 1,
            identifies: num(11, 1)? == 1,
            hire_list: num(12, 1)? == 1,
            resurrects: num(13, 1)? == 1,
        });
    }
    Ok(out)
}

// ------------------------------------------------------------ records

/// One NPC record. Only the fields this module owns; the store fields
/// belong to the vendors module.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NpcRecord {
    /// Monstats class.
    pub class: u16,
    /// Hire list.
    pub hire: Option<HireList>,
    /// Hire list made.
    pub hire_made: bool,
    /// Act 0–4.
    pub act: u8,
    /// Trader.
    pub trader: u8,
    /// NPC table byte 6.
    pub byte6: u8,
}

/// The NPC control block with its records, plus the table data this
/// module reads.
#[derive(Debug, Clone)]
pub struct NpcControl {
    /// The records in `monstats` row order.
    pub records: Vec<NpcRecord>,
    /// NPC-control seed.
    pub seed: Seed,
    /// GUID of the last NPC whose chat opened (−1 for none). Nothing
    /// reads it yet.
    pub last_chat_npc: u32,
    /// `monstats` flags (`npc`, `interact`) by class.
    flags: Vec<(bool, bool)>,
    /// `hireling` rows in table order.
    pub hirelings: Vec<HireRow>,
    /// The game is an expansion game.
    pub expansion: bool,
    /// Difficulty 0–2.
    pub difficulty: u8,
}

impl NpcControl {
    /// Built at game creation: steps the game seed once (the third seed
    /// derived at game creation).
    ///
    /// The item ids of `cqv` / `aqv` and the store data of traders belong
    /// to the vendors module, which attaches them per record.
    pub fn new(
        monstats: &[Monstats],
        hirelings: Vec<HireRow>,
        expansion: bool,
        difficulty: u8,
        game_seed: &mut Seed,
    ) -> Result<Self, NpcError> {
        let seed = Seed::init_low(game_seed.step());
        let table = parse_npc_table(VENDORS_TSV)?;
        let mut records = Vec::new();
        for (class, m) in monstats.iter().enumerate() {
            if !m.interact {
                continue;
            }
            if records.len() == RECORD_MAX {
                return Err(NpcError::TooManyRecords);
            }
            let class = class as u16;
            let mut r = NpcRecord {
                class,
                ..NpcRecord::default()
            };
            if let Some(t) = table.iter().find(|t| t.class == class) {
                r.act = t.act;
                r.trader = t.trader;
                r.byte6 = t.byte6;
            }
            records.push(r);
        }
        Ok(Self {
            records,
            seed,
            last_chat_npc: u32::MAX,
            flags: monstats.iter().map(|m| (m.npc, m.interact)).collect(),
            hirelings,
            expansion,
            difficulty,
        })
    }

    /// The first record of that class. No record has class 0 unless
    /// monstats row 0 has `interact`.
    pub fn record(&self, class: u16) -> Option<&NpcRecord> {
        self.records.iter().find(|r| r.class == class)
    }

    pub fn record_mut(&mut self, class: u16) -> Option<&mut NpcRecord> {
        self.records.iter_mut().find(|r| r.class == class)
    }

    /// `monstats` flag `npc` (bit 8) of a class.
    pub fn is_npc(&self, class: u16) -> bool {
        self.flags.get(usize::from(class)).is_some_and(|f| f.0)
    }

    /// `monstats` flag `interact` (bit 9) of a class.
    pub fn interacts(&self, class: u16) -> bool {
        self.flags.get(usize::from(class)).is_some_and(|f| f.1)
    }
}

// ------------------------------------------------------------ seams

/// The interaction list of an NPC (the list head of its interaction
/// block). Embedded in the monster data of `interact` monsters by the
/// monster code.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InteractionList {
    /// (player, state) from the list head.
    pub nodes: Vec<(UnitId, u8)>,
}

impl InteractionList {
    pub fn state(&self, player: UnitId) -> Option<u8> {
        self.nodes.iter().find(|n| n.0 == player).map(|n| n.1)
    }

    fn set_state(&mut self, player: UnitId, state: u8) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.0 == player) {
            n.1 = state;
        }
    }

    fn remove(&mut self, player: UnitId) {
        if let Some(i) = self.nodes.iter().position(|n| n.0 == player) {
            self.nodes.remove(i);
        }
    }
}

/// The seam to the rest of the game. Expected providers in brackets.
pub trait NpcWorld {
    // Game.
    /// The item format.
    fn item_format(&self) -> u16;

    // Units.
    fn guid(&self, unit: UnitId) -> u32;
    /// A monster by GUID.
    fn monster_by_guid(&self, guid: u32) -> Option<UnitId>;
    /// Any unit by GUID with its unit type (0x2F / 0x30 read only the
    /// GUID).
    fn unit_by_guid(&self, guid: u32) -> Option<(u8, UnitId)>;
    fn monster_class(&self, unit: UnitId) -> Option<u16>;
    /// Unit mode (NPC: 0 death, 12 dead).
    fn mode(&self, unit: UnitId) -> u8;
    fn set_mode(&mut self, unit: UnitId, mode: u8);
    fn clear_unit_flag(&mut self, unit: UnitId, flag: u32);
    /// Unit distance.
    fn distance(&self, a: UnitId, b: UnitId) -> i32;
    /// 0 when the player is within 50 subtiles of the NPC on both axes,
    /// else its result code.
    fn axis_check(&self, player: UnitId, npc: UnitId) -> u32;
    /// The NPC is in the player's act.
    fn same_act(&self, player: UnitId, npc: UnitId) -> bool;
    /// The unit check for a 0x38 NPC GUID; 0 = accept.
    fn unit_check(&self, player: UnitId, guid: u32) -> u32;
    /// 0 when the player is free (no interact unit, no cursor item, not
    /// busy).
    fn player_busy(&self, player: UnitId) -> u32;
    /// The start check of an interaction (not fully known yet).
    fn start_allowed(&self, player: UnitId, npc: UnitId) -> bool;
    /// Tristram Cain (cain1) is busy with the player's quest.
    fn tristram_cain_busy(&self, player: UnitId, npc: UnitId) -> bool;
    /// Clear a unit's path.
    fn clear_path(&mut self, unit: UnitId);
    /// Set the NPC's AI parameter 0 (its meaning is not fully known yet).
    fn npc_ai_param(&mut self, npc: UnitId, param: u32);
    /// Set AI params 1 and 2 of the NPC (C→S 0x59). Default: not
    /// written.
    fn npc_ai_point(&mut self, _npc: UnitId, _x: u32, _y: u32) {}
    /// Cancel the NPC's AI-think events (type 2) and schedule one at
    /// frame + 1.
    fn reschedule_ai_think(&mut self, npc: UnitId);
    /// Player movement toward the target.
    fn approach(&mut self, player: UnitId, npc: UnitId);
    /// The interaction list of an `interact` monster.
    fn interaction(&mut self, npc: UnitId) -> Option<&mut InteractionList>;
    /// The player's interact unit (type, GUID).
    fn interact_unit(&self, player: UnitId) -> Option<(u8, u32)>;
    /// Set the player's interact unit.
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32);
    /// Reset the player's interact unit (GUID −1, type 6, flag 0).
    fn reset_interact(&mut self, player: UnitId);
    /// The player's pet of that kind (kind 7 = hireling).
    fn pet(&self, player: UnitId, kind: u8, arg: u8) -> Option<UnitId>;
    /// Pets in the player's pet order.
    fn pets(&self, player: UnitId) -> Vec<UnitId>;

    // Stats and states.
    fn stat(&self, unit: UnitId, stat: u16) -> u32;
    /// The base (unit-list) value.
    fn base_stat(&self, unit: UnitId, stat: u16) -> u32;
    /// Set without a message.
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: u32);
    /// Set and send (SetStat 0x1D–0x1F).
    fn set_stat_send(&mut self, player: UnitId, stat: u16, value: u32);
    /// Max life, mana and stamina.
    fn max_life(&self, unit: UnitId) -> u32;
    fn max_mana(&self, unit: UnitId) -> u32;
    fn max_stamina(&self, unit: UnitId) -> u32;
    /// States count (`states` table).
    fn states_count(&self) -> u16;
    fn has_state(&self, unit: UnitId, state: u16) -> bool;
    /// The state's curable mask is set.
    fn curable(&self, state: u16) -> bool;
    fn has_state_list(&self, unit: UnitId, state: u16) -> bool;
    fn remove_state_list(&mut self, unit: UnitId, state: u16);
    /// Attach a sound event to the unit.
    fn attach_sound(&mut self, unit: UnitId, sound: u16);
    /// The sound event sent only to `target`'s client. Default: every
    /// client, the same bytes in a one-player game.
    fn attach_sound_to(&mut self, unit: UnitId, sound: u16, target: UnitId) {
        let _ = target;
        self.attach_sound(unit, sound);
    }

    // Messages.
    fn send(&mut self, player: UnitId, msg: &[u8]);
    /// Diagnostic notification when an NPC menu is opened.
    fn record_menu(&mut self, _class: u32) {}

    // Quests (the game's QuestControl).
    /// The player's flag record of the game's difficulty.
    fn quest_flags(&self, player: UnitId) -> QuestFlags;
    /// Quest event 0 for player and NPC (`QuestControl::npc_activate`);
    /// the text list it built.
    fn quest_text_list(&mut self, player: UnitId, npc: UnitId) -> TextList;
    /// The 34 list bytes of S→C 0x27 from byte 6: count u8@6, then
    /// (kind u8, string u16) at 8 + 4k for k < 8; the client refuses a
    /// count ≥ 8.
    fn encode_text_list(&self, list: &TextList) -> [u8; 34];
    /// S→C 0x29 (`QuestControl::send_game_flags`).
    fn send_game_quests(&mut self, player: UnitId);
    /// S→C 0x28 (`quests::send_player_flags`).
    fn send_player_quests(&mut self, player: UnitId, unit_type: u8, guid: u32);
    /// `QuestControl::npc_deactivate`.
    fn quest_chat_end(&mut self, player: UnitId, npc: UnitId);
    /// `quests::act1::respec_offer`.
    fn respec_offer(&mut self, player: UnitId);
    /// `quests::act1::respec_done`.
    fn respec_done(&mut self, player: UnitId);
    /// `quests::act1::imbue_granted`.
    fn imbue_granted(&mut self, player: UnitId);
    /// The Act V socket quest hook (not written yet).
    fn socket_granted(&mut self, player: UnitId);
    /// The Act V personalize quest hook (not written yet).
    fn personalize_granted(&mut self, player: UnitId);
    /// `QuestControl::act_completion(npc, level, from)`.
    fn act_completion(&mut self, player: UnitId, npc: UnitId, level: u32, from: u32);

    // Player.
    /// Stat reset.
    fn reset_stats(&mut self, player: UnitId);
    /// Skill reset.
    fn reset_skills(&mut self, player: UnitId);
    /// The respec sound.
    fn respec_sound(&mut self, player: UnitId);
    fn player_name(&self, player: UnitId) -> Vec<u8>;
    /// Act change to `level` with `arg`.
    fn act_change(&mut self, player: UnitId, level: u32, arg: u32);
    /// Activate the level's waypoint.
    fn activate_waypoint(&mut self, player: UnitId, level: u32);

    // Items.
    /// The player's items in inventory order.
    fn inventory(&self, player: UnitId) -> Vec<InvEntry>;
    /// Identify an item.
    fn identify(&mut self, item: UnitId);
    fn cursor_item(&self, player: UnitId) -> Option<UnitId>;
    fn item_facts(&self, item: UnitId) -> ItemFacts;
    /// Put the refused item back.
    fn put_back(&mut self, player: UnitId, item: UnitId);
    /// Remove the input from the cursor.
    fn remove_cursor_item(&mut self, player: UnitId, item: UnitId) -> bool;
    /// Duplicate an item into the player.
    fn duplicate(&mut self, player: UnitId, item: UnitId) -> Option<UnitId>;
    /// Fill a drop request from `input` (read before the input leaves the
    /// cursor), apply `mods`, create.
    fn create_imbued(&mut self, player: UnitId, input: UnitId, mods: &ImbueMods) -> Option<UnitId>;
    /// Refresh an item.
    fn item_refresh(&mut self, item: UnitId);
    fn set_item_page(&mut self, item: UnitId, page: u8);
    fn set_item_flag(&mut self, item: UnitId, flag: u32);
    fn personal_name(&self, item: UnitId) -> Vec<u8>;
    fn set_personal_name(&mut self, item: UnitId, name: &[u8]);
    /// Place in the inventory, or drop at a free spot near the player.
    fn place_or_drop(&mut self, player: UnitId, item: UnitId);
    /// The item's max sockets.
    fn max_sockets(&self, item: UnitId) -> u32;
    /// Add `n` sockets.
    fn add_sockets(&mut self, item: UnitId, n: u32);
    fn item_seed(&mut self, item: UnitId) -> &mut Seed;

    // Mercenaries.
    /// Spawn a mercenary near `near` (the hire, spread 4).
    fn spawn_mercenary(&mut self, near: UnitId, class: u32, mode: u8) -> Option<UnitId>;
    /// Spawn a mercenary near `near` with `spread`: the quest reward
    /// tries spreads 4, 6 and 12. Default: spread 4 only.
    fn spawn_mercenary_spread(
        &mut self,
        near: UnitId,
        class: u32,
        mode: u8,
        spread: i32,
    ) -> Option<UnitId> {
        (spread == 4)
            .then(|| self.spawn_mercenary(near, class, mode))
            .flatten()
    }
    /// Set up a hired mercenary.
    fn init_mercenary(&mut self, player: UnitId, merc: UnitId, init: &MercInit);
    /// Revive the player's mercenary.
    fn revive_mercenary(&mut self, player: UnitId, merc: UnitId);

    // Rules.
    /// The game's rules; a mod game's NPC roles and menu actions come from
    /// them ([`NpcProvider`]). Default: D2RUST's.
    fn rules(&self) -> &crate::rules::Ruleset {
        &crate::rules::ORIGINAL
    }
}

/// The vendors seam (module `world::vendors`).
pub trait NpcVendors {
    /// Open trade or gamble. Store generation draws from `ctl.seed`;
    /// asheara's trade open makes the hire list with
    /// [`NpcControl::make_hire_list`].
    fn open_trade(
        &mut self,
        ctl: &mut NpcControl,
        player: UnitId,
        npc: UnitId,
        single: bool,
        gamble: bool,
    ) -> Result<(), NpcError>;
    /// Drop the player's gamble list.
    fn drop_gamble_list(&mut self, player: UnitId, npc: UnitId);
    /// Take `cost` gold; false when not enough.
    fn pay(&mut self, player: UnitId, cost: u32) -> bool;
    /// Repair an item.
    fn repair(&mut self, item: UnitId);
}

// ------------------------------------------------------------ messages

/// S→C 0x2A (15 bytes). Bytes 3–6 are unused; D2RUST writes 0.
pub fn transaction(kind: u8, code: u8, guid: u32, gold: u32) -> [u8; 15] {
    let mut m = [0u8; 15];
    m[0] = 0x2A;
    m[1] = kind;
    m[2] = code;
    m[7..11].copy_from_slice(&guid.to_le_bytes());
    m[11..15].copy_from_slice(&gold.to_le_bytes());
    m
}

/// S→C 0x58 (7 bytes); byte 6 (`effect`) is only meaningful on the
/// object-insert result-5 path, so for the service results 6 / 7
/// D2RUST writes 0.
pub fn service_result(npc_guid: u32, result: u8) -> [u8; 7] {
    let mut m = [0u8; 7];
    m[0] = 0x58;
    m[1..5].copy_from_slice(&npc_guid.to_le_bytes());
    m[5] = result;
    m
}

/// S→C 0x9B after a resurrection.
pub fn resurrect_message() -> [u8; 7] {
    [0x9B, 0xFF, 0xFF, 0, 0, 0, 0]
}

/// 0x2A with kind 0 and GUID −1, gold = stat 14.
fn send_code<W: NpcWorld>(w: &mut W, player: UnitId, code: u8, guid: u32) {
    let gold = w.stat(player, stat::GOLD);
    w.send(player, &transaction(0, code, guid, gold));
}

fn u32_at(msg: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(msg[o..o + 4].try_into().expect("4 bytes"))
}

// ------------------------------------------------------------ talk, chat, menus

impl NpcControl {
    /// C→S 0x13 for a monster. `None`: a unit type other than 1 (not an
    /// NPC). Returns the result code.
    pub fn interact<W: NpcWorld + NpcVendors>(
        &mut self,
        w: &mut W,
        player: UnitId,
        msg: &[u8],
    ) -> Result<Option<u32>, NpcError> {
        if msg.len() != 9 {
            return Ok(Some(3));
        }
        let ty = u32_at(msg, 1);
        if ty > 5 {
            return Ok(Some(2));
        }
        if ty != u32::from(UNIT_MONSTER) {
            return Ok(None);
        }
        let guid = u32_at(msg, 5);
        let Some(npc) = w.monster_by_guid(guid) else {
            return Ok(Some(1));
        };
        let d = w.distance(player, npc);
        if d > START_DISTANCE {
            return Ok(Some(1));
        }
        let class = w.monster_class(npc).unwrap_or(0);
        crate::cov!(NpcTopic, class, 0);
        if self.is_npc(class) && self.interacts(class) {
            w.clear_path(npc);
            w.npc_ai_param(npc, AI_PARAM);
            w.reschedule_ai_think(npc);
        }
        if d > APPROACH_DISTANCE {
            return Ok(Some(0));
        }
        if d > TALK_DISTANCE {
            w.approach(player, npc);
            return Ok(Some(0));
        }
        if w.player_busy(player) != 0 {
            return Ok(Some(0));
        }
        w.clear_path(player);
        self.start(w, player, npc).map(Some)
    }

    /// C→S 0x59 MakeEntityMove. The size is 17, else result 3; a unit
    /// type (u32@1) above 5 gives 2; no such unit, a distance of 51 or
    /// more, or a monster whose class lacks `npc` and `interact` gives 1.
    /// Else the NPC's path is cleared, its AI think rescheduled at the
    /// next frame and its AI params 0, 1 and 2 set to 40, x (u32@9) and y
    /// (u32@13); the result is 0. Only a monster (type 1) is looked up
    /// here; another unit type is refused with 1.
    pub fn make_entity_move<W: NpcWorld + NpcVendors>(
        &mut self,
        w: &mut W,
        player: UnitId,
        msg: &[u8],
    ) -> u32 {
        if msg.len() != 17 {
            return 3;
        }
        let ty = u32_at(msg, 1);
        if ty > 5 {
            return 2;
        }
        if ty != u32::from(UNIT_MONSTER) {
            return 1;
        }
        let Some(npc) = w.monster_by_guid(u32_at(msg, 5)) else {
            return 1;
        };
        if w.distance(player, npc) >= 0x33 {
            return 1;
        }
        let class = w.monster_class(npc).unwrap_or(0);
        if !(self.is_npc(class) && self.interacts(class)) {
            return 1;
        }
        w.clear_path(npc);
        w.reschedule_ai_think(npc);
        w.npc_ai_param(npc, AI_PARAM);
        w.npc_ai_point(npc, u32_at(msg, 9), u32_at(msg, 13));
        0
    }

    /// Start an interaction.
    fn start<W: NpcWorld + NpcVendors>(
        &mut self,
        w: &mut W,
        player: UnitId,
        npc: UnitId,
    ) -> Result<u32, NpcError> {
        // Only the start check and "already in the list" failures name
        // a result (1); the others return 0.
        if w.interact_unit(player).is_some() {
            return Ok(0);
        }
        let mode = w.mode(npc);
        if mode == 0 || mode == 12 || w.player_busy(player) == 1 {
            return Ok(0);
        }
        if !w.start_allowed(player, npc) {
            return Ok(1);
        }
        let class = w.monster_class(npc).unwrap_or(0);
        if class == CAIN1 && w.tristram_cain_busy(player, npc) {
            return Ok(0);
        }
        // An NPC without an interaction list: nothing happens.
        let Some(list) = w.interaction(npc) else {
            return Ok(0);
        };
        if list.state(player).is_some() {
            return Ok(1);
        }
        let first = list.nodes.is_empty();
        list.nodes.insert(0, (player, talk::TALKING));
        self.send_hire_list(w, player, npc, first)?;
        let guid = w.guid(npc);
        w.set_interact(player, UNIT_MONSTER, guid);
        let list = w.quest_text_list(player, npc);
        let mut m = Vec::with_capacity(40);
        m.extend_from_slice(&[0x27, 1]);
        m.extend_from_slice(&guid.to_le_bytes());
        m.extend_from_slice(&w.encode_text_list(&list));
        w.send(player, &m);
        w.record_menu(u32::from(class));
        w.send_game_quests(player);
        w.send_player_quests(player, UNIT_MONSTER, guid);
        Ok(0)
    }

    /// The common checks of 0x2F and 0x30.
    fn chat_npc<W: NpcWorld>(w: &mut W, player: UnitId, msg: &[u8]) -> Result<UnitId, u32> {
        if msg.len() != 9 {
            return Err(3);
        }
        let Some((ty, npc)) = w.unit_by_guid(u32_at(msg, 5)) else {
            return Err(1);
        };
        if ty != UNIT_MONSTER || w.interaction(npc).is_none() {
            return Err(3);
        }
        if !w.same_act(player, npc) {
            return Err(2);
        }
        Ok(npc)
    }

    /// C→S 0x2F chat open.
    pub fn chat_open<W: NpcWorld>(&mut self, w: &mut W, player: UnitId, msg: &[u8]) -> u32 {
        let npc = match Self::chat_npc(w, player, msg) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let r = w.axis_check(player, npc);
        if r != 0 {
            return r;
        }
        let list = w.interaction(npc).expect("checked");
        if list.state(player) != Some(talk::TALKING) {
            return 0;
        }
        list.set_state(player, talk::CHATTING);
        self.heal_hook(w, player, npc);
        0
    }

    /// C→S 0x30 chat close.
    pub fn chat_close<W: NpcWorld + NpcVendors>(
        &mut self,
        w: &mut W,
        player: UnitId,
        msg: &[u8],
    ) -> u32 {
        let npc = match Self::chat_npc(w, player, msg) {
            Ok(n) => n,
            Err(r) => return r,
        };
        w.quest_chat_end(player, npc);
        let list = w.interaction(npc).expect("checked");
        let Some(state) = list.state(player) else {
            return 0;
        };
        list.remove(player);
        let empty = list.nodes.is_empty();
        if w.interact_unit(player)
            .is_some_and(|(t, _)| t == UNIT_MONSTER)
        {
            w.reset_interact(player);
        }
        if state >= talk::CHATTING && empty {
            w.drop_gamble_list(player, npc);
        }
        0
    }

    /// C→S 0x38 menu action. No 0x2A is sent here.
    pub fn menu_action<W: NpcWorld + NpcVendors>(
        &mut self,
        w: &mut W,
        player: UnitId,
        msg: &[u8],
    ) -> Result<u32, NpcError> {
        if msg.len() != 13 {
            return Ok(3);
        }
        let (action, guid, item) = (u32_at(msg, 1), u32_at(msg, 5), u32_at(msg, 9));
        let r = w.unit_check(player, guid);
        if r != 0 {
            return Ok(r);
        }
        let Some(npc) = w.monster_by_guid(guid) else {
            return Ok(0);
        };
        let class = w.monster_class(npc).unwrap_or(0);
        crate::cov!(NpcTopic, class, (1u32 << 16) | (action & 0xFFFF));
        if !self.interacts(class) {
            return Ok(0);
        }
        let Some(list) = w.interaction(npc) else {
            return Ok(0);
        };
        let count = list.nodes.len();
        let single = count == 1;
        // The mod's action first; `None` runs D2RUST's.
        let rules = w.rules().clone();
        if let Some(p) = rules.npcs() {
            if let Some(r) = p.menu_action(self, w, player, npc, class, action, item) {
                return r;
            }
        }
        let roles = NpcRoles::of(&rules, class);
        match action {
            1 | 2 => {
                let gamble = action == 2;
                let allowed = if gamble { roles.gambles } else { roles.trades };
                if allowed {
                    // State 1 → 2; skipped when the player has no
                    // node.
                    if let Some(list) = w.interaction(npc) {
                        if list.state(player) == Some(talk::CHATTING) {
                            list.set_state(player, talk::TRADING);
                        }
                    }
                    w.open_trade(self, player, npc, single, gamble)?;
                }
            }
            3 => self.send_hire_list(w, player, npc, count < 2)?,
            _ => self.service(w, player, npc, class, action, item),
        }
        Ok(0)
    }

    /// The NPC services: menu actions other than 1, 2 and 3.
    fn service<W: NpcWorld + NpcVendors>(
        &mut self,
        w: &mut W,
        player: UnitId,
        npc: UnitId,
        class: u16,
        action: u32,
        item: u32,
    ) {
        match class {
            CHARSI | LARZUK | DREHYA => services::item_service(w, player, npc, class, item),
            AKARA => services::respec(w, player, self.difficulty),
            _ => services::act_travel(w, player, npc, class, action, self.expansion),
        }
    }

    // ------------------------------------------------------------ healing

    /// The heal hook of a chat open.
    fn heal_hook<W: NpcWorld>(&mut self, w: &mut W, player: UnitId, npc: UnitId) {
        let guid = w.guid(npc);
        self.last_chat_npc = guid;
        let class = w.monster_class(npc).unwrap_or(0);
        if NpcRoles::of(w.rules(), class).heals {
            heal(w, player, npc);
        }
    }

    // ------------------------------------------------------------ identify

    /// C→S 0x34 Cain identify.
    pub fn identify<W: NpcWorld + NpcVendors>(
        &mut self,
        w: &mut W,
        player: UnitId,
        msg: &[u8],
    ) -> u32 {
        if msg.len() != 5 {
            return 3;
        }
        let guid = u32_at(msg, 1);
        let Some(npc) = w.monster_by_guid(guid) else {
            send_code(w, player, code::REFUSED, u32::MAX);
            return 0;
        };
        if w.interact_unit(player) != Some((UNIT_MONSTER, guid)) {
            send_code(w, player, code::REFUSED, u32::MAX);
            return 0;
        }
        let class = w.monster_class(npc).unwrap_or(0);
        if !NpcRoles::of(w.rules(), class).identifies {
            return 0;
        }
        let items: Vec<UnitId> = w
            .inventory(player)
            .into_iter()
            .filter(|e| e.flags & 0x10 == 0)
            .filter(|e| matches!(e.place, Place::Grid(0) | Place::Grid(3) | Place::Equipped))
            .map(|e| e.item)
            .collect();
        if items.is_empty() {
            send_code(w, player, code::REFUSED, u32::MAX);
            return 0;
        }
        let f = w.quest_flags(player);
        if !(f.get(4, 0) || f.get(4, 1)) {
            let cost = 100u32.wrapping_mul(items.len() as u32);
            if !w.pay(player, cost) {
                send_code(w, player, code::NO_GOLD, u32::MAX);
                return 0;
            }
        }
        for item in items {
            w.identify(item);
            // Each identify sends the identify sound to the player's
            // client; the last of the loop is the one heard.
            w.attach_sound_to(player, SOUND_IDENTIFY, player);
        }
        send_code(w, player, code::IDENTIFIED, u32::MAX);
        0
    }
}

/// Heal a player at a healer.
fn heal<W: NpcWorld>(w: &mut W, player: UnitId, npc: UnitId) {
    let guid = w.guid(npc);
    if w.interact_unit(player) != Some((UNIT_MONSTER, guid)) {
        return;
    }
    let mut changed = false;
    let maxes = [
        (stat::LIFE, w.max_life(player)),
        (stat::MANA, w.max_mana(player)),
        (stat::STAMINA, w.max_stamina(player)),
    ];
    for (s, max) in maxes {
        if w.stat(player, s) < max {
            w.set_stat_send(player, s, max);
            changed = true;
        }
    }
    changed |= remove_list(w, player, 2);
    changed |= remove_list(w, player, 1);
    changed |= cure_states(w, player);
    for pet in w.pets(player) {
        // Life to max only when below max, counted as a change.
        let max = w.max_life(pet);
        if w.stat(pet, stat::LIFE) < max {
            w.set_stat(pet, stat::LIFE, max);
            changed = true;
        }
        changed |= cure_states(w, pet);
        changed |= remove_list(w, pet, 2);
        changed |= remove_list(w, pet, 1);
    }
    if changed {
        w.attach_sound(npc, SOUND_HEAL);
    }
}

fn remove_list<W: NpcWorld>(w: &mut W, unit: UnitId, state: u16) -> bool {
    if w.has_state_list(unit, state) {
        w.remove_state_list(unit, state);
        return true;
    }
    false
}

/// Remove every curable state's stat list.
fn cure_states<W: NpcWorld>(w: &mut W, unit: UnitId) -> bool {
    let mut changed = false;
    for s in 0..w.states_count() {
        if w.has_state(unit, s) && w.curable(s) && w.has_state_list(unit, s) {
            w.remove_state_list(unit, s);
            changed = true;
        }
    }
    changed
}

impl HireRow {
    /// Decodes the `hireling` table (fixed up: name ids at bytes
    /// 0x114 / 0x116).
    pub fn from_table(t: &BinTable) -> Result<Vec<HireRow>, NpcError> {
        if t.name != Hireling::TABLE || t.record_size != Hireling::SIZE {
            return Err(NpcError::Table(format!(
                "{} ({}-byte records) is not hireling",
                t.name, t.record_size
            )));
        }
        Ok(t.iter()
            .map(|r| {
                let h = Hireling::decode(r);
                HireRow {
                    version: h.version,
                    class: h.class,
                    act: h.act,
                    difficulty: h.difficulty,
                    seller: h.seller,
                    gold: h.gold,
                    level: h.level,
                    name_first: u16::from_le_bytes([r[0x114], r[0x115]]),
                    name_last: u16::from_le_bytes([r[0x116], r[0x117]]),
                }
            })
            .collect())
    }
}
