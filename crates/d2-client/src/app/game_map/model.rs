//! The `game-map.json` model: objectives, the world graph, playables and the
//! data-derived interaction matrix. Every field comes from the game's tables
//! or from `game-map.overrides.toml`; nothing here is base-game specific.

use serde::{Deserialize, Serialize};

/// The map format word.
pub const FORMAT: &str = "game-map 1";

/// One generated map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameMap {
    pub format: String,
    pub game: GameInfo,
    pub summary: Summary,
    pub objectives: Objectives,
    pub world: World,
    pub playables: Playables,
    pub mechanics: Mechanics,
}

/// Where the tables came from and which overrides were applied.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameInfo {
    pub source: String,
    pub mod_name: Option<String>,
    pub overrides: Option<String>,
    pub final_objective: Option<String>,
}

/// Counts a reader sees at a glance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub acts: u32,
    pub areas: u32,
    pub links: u32,
    pub waypoints: u32,
    pub towns: u32,
    pub quests: u32,
    pub boss_kills: u32,
    pub classes: u32,
    pub skills: u32,
    pub items: u32,
    pub npcs: u32,
    pub objects: u32,
    pub cube_recipes: u32,
    pub runewords: u32,
}

/// Quests, boss kills, act transitions and the endgame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Objectives {
    pub quests: Vec<Quest>,
    pub boss_kills: Vec<BossKill>,
    pub act_transitions: Vec<ActTransition>,
    pub endgame: Option<Endgame>,
}

/// One quest row with the steps the data names.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Quest {
    pub index: u32,
    pub chain: u32,
    pub act: u32,
    pub name: String,
    pub flag: Option<u32>,
    pub version: Option<u32>,
    /// The quest's NPC message table, when the data names one. Some quests
    /// complete through scripted events and have no NPC step at all.
    pub message_table: Option<u32>,
    pub steps: Vec<QuestStep>,
    pub rewards: Vec<Reward>,
    pub prerequisites: Vec<u32>,
}

/// One quest state: the NPC messages that advance it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QuestStep {
    pub state: u32,
    pub npcs: Vec<u32>,
    pub menus: Vec<u32>,
    pub messages: Vec<u32>,
    /// The (NPC, message) pairs this state names, so a goal can send the
    /// NPC's own quest line (C→S 0x31).
    pub npc_messages: Vec<NpcMessage>,
    pub text: String,
}

/// One NPC line of a quest state.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NpcMessage {
    pub npc: u32,
    pub message: u32,
}

/// A reward named by the overrides.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reward {
    pub kind: String,
    pub name: String,
    pub item: Option<String>,
}

/// A boss in a level. Where a boss stands is in the level maps, not the
/// tables, so the overrides' `[[boss]]` list names each one: a
/// `superuniques` row (its class comes from the row) or a monster class.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BossKill {
    pub name: String,
    pub class: u32,
    pub area: u32,
    pub act: u32,
    /// The `superuniques` row, for a superunique boss.
    pub superunique: Option<u32>,
    /// Scenario steps (as after `at T`) the goal runs before the walk to
    /// the boss: the seal, altar or object the boss is released by.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<String>,
    /// The goal enters the level from its act's town (a boss the game only
    /// releases after a level-change event).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_town: bool,
    /// Ticks from the walk to the boss to the goal's end, for a boss whose
    /// life outlasts the default fight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fight: Option<u32>,
    /// Why the boss goal is expected to block (a known open defect); a
    /// pending goal that blocks does not fail the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<String>,
}

/// One act-to-act transition named by the overrides.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActTransition {
    pub from: u32,
    pub to: u32,
    pub quest: Option<String>,
    pub area: Option<u32>,
}

/// The final objective named by the overrides.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Endgame {
    pub name: String,
    pub area: Option<u32>,
    pub boss: Option<u32>,
}

/// The world graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct World {
    pub areas: Vec<Area>,
    pub links: Vec<Link>,
    pub waypoints: Vec<Waypoint>,
    pub towns: Vec<u32>,
    /// Areas the shipped game has no entrance to, with the override's reason.
    pub unreachable: Vec<UnreachableArea>,
}

/// One area the overrides mark as unreachable by design.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UnreachableArea {
    pub area: u32,
    pub reason: String,
}

/// One level.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub id: u32,
    pub name: String,
    pub act: u32,
    pub waypoint: bool,
    pub town: bool,
    pub size: [u32; 2],
    pub monster_levels: [u32; 3],
}

/// One edge between two levels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub from: u32,
    pub to: u32,
    pub kinds: Vec<String>,
    pub portal: bool,
    pub gate: Option<String>,
    /// Why the override adds an edge the tables cannot name.
    pub reason: Option<String>,
}

/// A waypoint area.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Waypoint {
    pub level: u32,
    pub act: u32,
    pub town: bool,
    pub name: String,
}

/// Classes, skills, items, NPCs, objects, cube recipes and runewords.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Playables {
    pub classes: Vec<Class>,
    pub skills: Vec<Skill>,
    pub items: Vec<Item>,
    pub npcs: Vec<Npc>,
    pub objects: Vec<GameObject>,
    pub cube_recipes: Vec<CubeRecipe>,
    pub runewords: Vec<Runeword>,
}

/// One playable class.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Class {
    pub id: u32,
    pub name: String,
    pub skills: Vec<u32>,
}

/// One skill row.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    pub id: u32,
    pub name: String,
    pub class: u32,
    pub req_level: u32,
    pub max_level: u32,
    pub mana: u32,
    pub mana_per_level: u32,
    pub passive: bool,
    pub aura: bool,
    pub target: String,
    pub requires: Vec<u32>,
}

/// One item row from weapons, armor or misc.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub code: String,
    pub name: String,
    pub kind: String,
    /// The item's direct type code.
    pub item_type: String,
    /// The direct type and its `Equiv1`/`Equiv2` parents: a sword also names
    /// its melee and weapon categories, a shield its shield and armor ones.
    pub types: Vec<String>,
    pub slot: Option<String>,
    pub max_sockets: u32,
    pub level_req: u32,
    pub level: u32,
    pub stackable: bool,
}

/// One NPC class and the services the engine's vendor data names.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Npc {
    pub class: u32,
    pub name: String,
    pub act: u32,
    pub services: Vec<String>,
    pub quest_chains: Vec<u32>,
}

/// One object class.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameObject {
    pub class: u32,
    pub token: String,
    pub name: String,
    pub kind: String,
    pub act: u32,
    pub door: bool,
    pub portal: bool,
    pub lockable: bool,
}

/// One cube recipe with its decoded slots.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CubeRecipe {
    pub index: u32,
    pub enabled: bool,
    pub min_difficulty: u32,
    pub num_inputs: u32,
    pub inputs: Vec<CubeInput>,
    pub outputs: Vec<CubeOutput>,
}

/// One cube input slot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CubeInput {
    pub kind: String,
    pub code: Option<String>,
    pub item_type: Option<String>,
    pub quality: u32,
    pub quantity: u32,
}

/// One cube output slot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CubeOutput {
    pub kind: String,
    pub code: Option<String>,
    pub item_type: Option<String>,
    pub quality: u32,
    pub quantity: u32,
}

/// One runeword row.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Runeword {
    pub name: String,
    pub complete: bool,
    pub runes: Vec<String>,
    pub item_types: Vec<String>,
}

/// Actors x actions x targets.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mechanics {
    pub interactions: Vec<Interaction>,
}

/// One interaction the data names.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Interaction {
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail: String,
}

/// A code field (4 bytes, NUL or space padded) as text.
pub fn code_text(code: &[u8; 4]) -> String {
    let end = code
        .iter()
        .rposition(|&b| b != 0 && b != b' ')
        .map_or(0, |i| i + 1);
    String::from_utf8_lossy(&code[..end]).into_owned()
}

/// A fixed byte string as text.
pub fn bytes_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

/// A link field: `0xFFFF_FFFF` and `0xFFFF` are "none".
pub fn link_u32(value: u32) -> Option<u32> {
    (value != u32::MAX).then_some(value)
}

/// A 16-bit link field: `0xFFFF` is "none".
pub fn link_u16(value: u16) -> Option<u32> {
    (value != u16::MAX).then_some(u32::from(value))
}
