//! The single-player session sequence of a client whose player is not yet
//! placed: the game-creation messages of C→S 0x67 ([`create_game`]) and the
//! join of C→S 0x6B ([`enter_game`]). The app builds its game directly (no
//! C→S 0x67 / 0x6B path); these are the game parts of both handlers.
//! [`super::session_flow`] drives them from the host's drain, and the app
//! and the server host call [`enter_game_from_save`] to put a loaded
//! character into the world.
//!
//! [`create_game`] queues, to the client's player: S→C 0x01 GameFlags,
//! 0x00 (client state 1), 0x02.
//!
//! [`enter_game`] queues, in this order:
//!
//! 1. the player's own add messages: S→C 0x59 AssignPlayer (GUID, class,
//!    name, (0, 0): the player is placed nowhere yet), then part B: 0xAA
//!    (its states), 0x76; then, for a loaded save ([`Entry::base_skills`]),
//!    the skills section's S→C 0x94 BaseSkillLevels from the player's
//!    server skill list; a new character's stub load reads no skills
//!    section, so it sends none (the client's own player init gives it its
//!    native skills);
//! 2. S→C 0x0B GameHandshake (type 0, the player's GUID);
//! 3. S→C 0x5F PortalFlags, with the player record;
//! 4. the player's stat messages (the mod-array flush, each value sent as
//!    0x1D, 0x1E or 0x1F by its size: [`stat_messages`]);
//! 5. S→C 0x7B for each hot-key slot whose skill is a `skills` row;
//! 6. two S→C 0x23 SetSkill, hand 1 then hand 0, with the player record;
//!    then the stat messages again;
//! 7. the join's vitals sync: S→C 0x95, then the gold and experience
//!    messages against the client's cache;
//! 8. S→C 0x03 LoadAct: the act, the act DRLG's init seed, the act's town
//!    level id, the object seed (`dwObjSeed`), then S→C 0x53 with the
//!    act's environment record ([`d2_sim::world::environment`]); client
//!    state 2;
//! 9. game entry: S→C 0x07 for the spawn room of the act's town, the room
//!    switch (0x07 and the add messages for every room of the spawn
//!    room's adjacency array), the player put in the world, S→C 0x15 with
//!    flag 1, S→C 0x7E (`d2_sim::wiring::path::place::game_entry`);
//! 10. client state 3: the next tick's client pass sends 0x04 once the
//!     client's room is ready.
//!
//! The messages go through the action wiring's transport seam
//! (`Pending::send`), so the host queues them with the next tick's
//! messages, before them.
//!
//! The values the join takes from the character (the save: name, act,
//! hot keys, the player record's portal flags and skill hands) are
//! [`Entry`]'s. A new character ([`Entry::new_character`]) has its record,
//! so it gets 0x5F and the two 0x23 (item 0), after the load's own 0x23; a
//! caller without a record gets no 0x5F and no 0x23.
//!
//! The loader's item calls' S→C 0x22 / 0x21 follow 0x94 and precede the
//! stub load's 0x23, in the save's item order.
//!
//! Not sent (named here, not guessed):
//! - the loader's other messages after 0x94 (0x23, 0x5E, 0x28, 0x29 from
//!   the loader's callees; the 0x23 of the stub load is sent);
//! - the player's item messages beyond the loader's, and the update-list
//!   reset (the item world is not reachable from the session), and the
//!   extra step of an expansion game;
//! - followers.

use d2_formats::d2s::D2s;
use d2_proto::server::LoadAct;
use d2_sim::drlg::TOWN_LEVELS;
use d2_sim::units::lists::client_state;
use d2_sim::units::messages as msg;
use d2_sim::units::UnitId;
use d2_sim::wiring::action::switch::assign_player;
use d2_sim::wiring::action::vitals_sync;
use d2_sim::wiring::action::Pending;
use d2_sim::wiring::path::place::game_entry;
use d2_sim::wiring::path::walk::PathCtx;

use super::character::{self, ActionCharacter, CharacterWorld, LoadContext, LoadError, LoadReport};
use super::handlers::world::Outbox;
use super::handlers::world::{ActionEvents, StartItems, WiredWorld};
use super::SimGame;
use crate::seams::ClientId;
use crate::seams::MessageSink;

/// The game fields of S→C 0x01.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameSetup {
    /// The game's difficulty: 0 Normal, 1 Nightmare, 2 Hell.
    pub difficulty: u8,
    /// The flags of the game's arena record.
    pub arena_flags: u32,
    /// The game is an expansion game.
    pub expansion: bool,
    /// The game is a ladder game.
    pub ladder: bool,
}

/// One hot-key slot of the client record (8 bytes per slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotKey {
    /// i16 at +0; a slot is sent only when 0 ≤ skill < the `skills` count.
    pub skill: i16,
    /// u8 at +2.
    pub flag: bool,
    /// u32 at +4.
    pub item: u32,
}

/// An unassigned slot (skill −1).
pub const NO_HOT_KEY: HotKey = HotKey {
    skill: -1,
    flag: false,
    item: u32::MAX,
};

/// One hand's skill of the player record (hand 0 is the right mouse
/// button, hand 1 the left; skill and owning item), sent as one S→C 0x23.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillHand {
    /// The skill id selected in this hand.
    pub skill: u16,
    /// The GUID of the item that owns the skill, or `u32::MAX` for none.
    pub item: u32,
}

/// The player record's fields the join sends (from the save).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerRecord {
    /// The portal flags of S→C 0x5F.
    pub portal_flags: u32,
    /// The skills of hand 0 and hand 1: the two S→C 0x23.
    pub hands: [SkillHand; 2],
}

/// What the joining character brings (its save).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The client's act byte: 0 for a new character.
    pub act: u8,
    /// The character name, zero-padded (player data; 0x59 bytes 6..22).
    pub name: [u8; 16],
    /// The client's hot-key slots.
    pub hotkeys: [HotKey; 16],
    /// The player record's values (a new character's:
    /// [`PlayerRecord::new_character`]). `None`: not given, so 0x5F and
    /// the two 0x23 are not sent.
    pub record: Option<PlayerRecord>,
    /// The new-character load's own right-skill selection (hand 0, the
    /// class's `StartSkill`, owner −1): one S→C 0x23 sent right after the
    /// add messages.
    pub load_skill: Option<SkillHand>,
    /// The load read a skills section (a full save): its S→C 0x94 follows
    /// the add messages. False for the new-character stub (it reads no
    /// section) and a caller without a load.
    pub base_skills: bool,
    /// The full-save load's post-load selections: the left class skill,
    /// then the right one, each one S→C 0x23 (owner −1) right after the add
    /// messages.
    pub load_selects: [Option<u16>; 2],
    /// The same for item-owned mouse skills ([`LoadReport::mouse_item_selected`]).
    pub load_item_selects: [Option<u16>; 2],
}

impl Entry {
    /// A character with no hot key set and no player-record values.
    pub fn new(act: u8, name: [u8; 16]) -> Self {
        Self {
            act,
            name,
            hotkeys: [NO_HOT_KEY; 16],
            record: None,
            load_skill: None,
            base_skills: false,
            load_selects: [None; 2],
            load_item_selects: [None; 2],
        }
    }

    /// A loaded full save's entry: act `act`, and the skills section's
    /// S→C 0x94 ([`Entry::base_skills`]).
    pub fn loaded(
        act: u8,
        name: [u8; 16],
        selects: [Option<u16>; 2],
        item_selects: [Option<u16>; 2],
    ) -> Self {
        Self {
            base_skills: true,
            load_selects: selects,
            load_item_selects: item_selects,
            ..Self::new(act, name)
        }
    }

    /// A new character (the 335-byte stub's load): act 0, no hot keys, the
    /// record of [`PlayerRecord::new_character`] and, when the right
    /// skill was selected, the load's own 0x23 (hand 0, the skill, item
    /// −1: a class skill has no owner item).
    pub fn new_character(name: [u8; 16], portal_levels: &[u32], right: Option<u16>) -> Self {
        Self {
            record: Some(PlayerRecord::new_character(portal_levels, right)),
            load_skill: right.map(|skill| SkillHand {
                skill,
                item: u32::MAX,
            }),
            ..Self::new(0, name)
        }
    }
}

/// The portal flags of a freshly allocated player:
/// `1 << i` with `i` the position of level 1 in the portal level list, 0
/// when absent (with the usual list, 1).
pub fn initial_portal_flags(portal_levels: &[u32]) -> u32 {
    portal_levels
        .iter()
        .position(|&l| l == 1)
        .and_then(|i| 1u32.checked_shl(i as u32))
        .unwrap_or(0)
}

impl PlayerRecord {
    /// A new character's record: the portal flags from [`initial_portal_flags`];
    /// hand 0 = the right skill `StartSkill` when the new-character load
    /// selected it, else 0; hand 1 = 0. Both items are 0
    /// (`23 00 <guid> 01 0000 00000000` and the hand 0 form).
    pub fn new_character(portal_levels: &[u32], right: Option<u16>) -> Self {
        Self {
            portal_flags: initial_portal_flags(portal_levels),
            hands: [
                SkillHand {
                    skill: right.unwrap_or(0),
                    item: 0,
                },
                SkillHand { skill: 0, item: 0 },
            ],
        }
    }
}

/// Why the join could not run.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum JoinError {
    /// The client has no client record.
    #[error("client {0} has not joined")]
    NotJoined(ClientId),
    /// The client record has no player unit.
    #[error("client {0} has no player unit")]
    NoPlayer(ClientId),
    /// The player is already in a room.
    #[error("player {0:?} is already in a room (game entry places an unplaced player)")]
    Placed(UnitId),
    /// The act has no dungeon layout.
    #[error("act {0} has no DRLG (the act is created by the game creation path)")]
    NoAct(u8),
    /// The path provider is not installed.
    #[error("the path provider is off (game entry places through it)")]
    NoPaths,
    /// A fatal failure of game entry (no spawn room, no free point), as
    /// the wiring logged it.
    #[error("game entry failed: {0}")]
    Entry(String),
    /// The save's load failed.
    #[error("character load: {0}")]
    Load(#[from] LoadError),
}

/// The S→C 0x03 LoadAct of act `act`, with the act's map and object seeds.
pub fn load_act(act: u8, map_seed: u32, obj_seed: u32) -> LoadAct {
    let town = TOWN_LEVELS
        .get(usize::from(act))
        .copied()
        .unwrap_or_default();
    LoadAct {
        act,
        f2: map_seed,
        f6: town as u16,
        f8: obj_seed,
    }
}

/// The stat message for stat `s` with value `v`: `v` as an unsigned u32
/// below 0xFF → S→C 0x1D (3 bytes), below 0xFFFF → 0x1E (4 bytes), else
/// 0x1F (6 bytes). `None` for s > 0xFE, which the wire cannot carry.
pub fn stat_message(stat: u16, value: i32) -> Option<Vec<u8>> {
    let s = u8::try_from(stat).ok().filter(|&s| s <= 0xFE)?;
    let v = value as u32;
    Some(if v < 0xFF {
        vec![0x1D, s, v as u8]
    } else if v < 0xFFFF {
        let w = (v as u16).to_le_bytes();
        vec![0x1E, s, w[0], w[1]]
    } else {
        let d = v.to_le_bytes();
        vec![0x1F, s, d[0], d[1], d[2], d[3]]
    })
}

/// The stat messages of the mod-array flush: one [`stat_message`] per
/// (key, base value) of `StatLists::mod_values`, in key order; the key's
/// layer is not sent. A stat id above 0xFE sends nothing; the `Saved`
/// stats are 0–15, so the array never holds one.
pub fn stat_messages(values: &[(i32, i32)]) -> Vec<Vec<u8>> {
    values
        .iter()
        .filter_map(|&(k, v)| stat_message(d2_sim::stats::key_stat(k), v))
        .collect()
}

/// The game-creation messages of `client`: S→C 0x01, 0x00 (client
/// state := 1), 0x02, to the client's player.
pub fn create_game<D: ActionEvents, W>(
    s: &mut SimGame<D, W>,
    client: ClientId,
    setup: &GameSetup,
) -> Result<(), JoinError> {
    let id = s.sim_client(client).ok_or(JoinError::NotJoined(client))?;
    let player = s.player_of(client).ok_or(JoinError::NoPlayer(client))?;
    let x = &mut s.events.action().sys.hooks.x;
    x.send(
        player,
        &msg::game_flags(
            setup.difficulty,
            setup.arena_flags,
            setup.expansion,
            setup.ladder,
        ),
    );
    x.send(player, &msg::GAME_LOADING);
    if let Some(e) = s.game.lists.client_mut(id) {
        e.state = client_state::LOADING;
    }
    s.events
        .action()
        .sys
        .hooks
        .x
        .send(player, &msg::LOAD_SUCCESSFUL);
    Ok(())
}

/// The skill levels a refused load had applied before it failed
/// (`d2_formats::d2s::skills_before_failure`): the player's
/// native skills (`player_skills`) and one `add_skill_level` per non-zero
/// byte, as [`character::load`] does for a full save.
pub fn apply_skills_before_failure<D: ActionEvents, W>(
    s: &mut SimGame<D, W>,
    player: UnitId,
    skills: &[u8],
) {
    s.events.action().with(&mut s.game, |_, v| {
        let mut cw = ActionCharacter { v, player };
        let _ = cw.player_skills();
        for (i, &level) in skills.iter().enumerate() {
            if level != 0 {
                let _ = cw.add_skill_level(i, level);
            }
        }
    });
}

/// The player's own add messages of a join (S→C 0x59 and its part B), sent
/// when the load then refuses the save after the player unit exists: the
/// stream of a section error holds them before the 0xB4.
pub fn announce_player<D: ActionEvents, W>(
    s: &mut SimGame<D, W>,
    client: ClientId,
    name: &[u8; 16],
    base_skills: bool,
    out: &mut dyn MessageSink,
) -> Result<(), JoinError>
where
    D::X: Outbox,
{
    let player = s.player_of(client).ok_or(JoinError::NoPlayer(client))?;
    let a = s.events.action();
    let (class, guid) = a
        .sys
        .units
        .get(player)
        .map(|r| (r.class, r.guid))
        .ok_or(JoinError::NoPlayer(client))?;
    a.sys
        .hooks
        .x
        .send(player, &assign_player(guid, class as u8, name, 0, 0));
    a.with(&mut s.game, |g, v| v.player_part_b(g, player, player));
    // The skills section was read before the failure: S→C 0x94, as in the
    // join.
    if base_skills {
        let m = a
            .sys
            .hooks
            .skill_lists
            .get(&player)
            .and_then(|l| msg::base_skill_levels(guid, &join_base_levels(l)));
        if let Some(m) = m {
            a.sys.hooks.x.send(player, &m);
        }
    }
    // Queued now: the client is removed right after (the outbox would
    // find no client).
    let sent = s.events.action().hooks().x.take_sent();
    for (unit, bytes) in sent {
        if let Some(c) = s.client_of(unit) {
            // A full queue loses the message.
            let _ = out.queue(c, &bytes);
        }
    }
    Ok(())
}

/// What the game entry and a player's leave need of the item world beyond
/// the session's own state: the loaded Iron Golem item equipped on the
/// re-cast golem, and a leaving player's belongings freed. Worlds without
/// an item model do nothing.
pub trait GolemWorld {
    /// Equips the loaded Iron Golem item on the golem just re-cast for
    /// `player`; returns one text per fault. The default does nothing.
    fn place_golem_item<D: ActionEvents>(
        &mut self,
        _game: &mut d2_sim::game::Game,
        _events: &mut D,
        _player: UnitId,
    ) -> Vec<String> {
        Vec::new()
    }

    /// Frees what a leaving `player` owns in this world: its items (with
    /// their socketed fillers) and its mercenary with the mercenary's
    /// items. The player unit itself stays for the caller to remove.
    /// Returns one text per fault. The default does nothing.
    fn free_player_belongings<D: ActionEvents>(
        &mut self,
        _game: &mut d2_sim::game::Game,
        _events: &mut D,
        _player: UnitId,
    ) -> Vec<String> {
        Vec::new()
    }
}

impl GolemWorld for super::handlers::world::NoWorld {}
impl<S> GolemWorld for super::handlers::world::ActionWorld<S> {}
impl<R, S> GolemWorld for WiredWorld<R, S> {
    fn place_golem_item<D: ActionEvents>(
        &mut self,
        game: &mut d2_sim::game::Game,
        events: &mut D,
        player: UnitId,
    ) -> Vec<String> {
        WiredWorld::place_golem_item(self, game, events, player)
    }

    fn free_player_belongings<D: ActionEvents>(
        &mut self,
        game: &mut d2_sim::game::Game,
        events: &mut D,
        player: UnitId,
    ) -> Vec<String> {
        WiredWorld::free_player_belongings(self, game, events, player)
    }
}

/// The join of `client` (module docs). Returns the placed player.
///
/// The object seed sent is the object control's; a game built without
/// an object control (not created through `WorldSim::create_game`) has
/// none, and 0 is sent.
pub fn enter_game<D: ActionEvents, W: GolemWorld>(
    s: &mut SimGame<D, W>,
    client: ClientId,
    entry: &Entry,
) -> Result<UnitId, JoinError> {
    let id = s.sim_client(client).ok_or(JoinError::NotJoined(client))?;
    let player = s.player_of(client).ok_or(JoinError::NoPlayer(client))?;
    if s.game.lists.unit(player).and_then(|u| u.room()).is_some() {
        return Err(JoinError::Placed(player));
    }
    let a = s.events.action();
    if a.sys.hooks.paths.is_none() {
        return Err(JoinError::NoPaths);
    }
    let map_seed = a
        .sys
        .hooks
        .drlg
        .dungeon
        .acts
        .get(usize::from(entry.act))
        .and_then(Option::as_ref)
        .map(|d| d.init_seed)
        .ok_or(JoinError::NoAct(entry.act))?;
    let (class, guid) = a
        .sys
        .units
        .get(player)
        .map(|r| (r.class, r.guid))
        .ok_or(JoinError::NoPlayer(client))?;
    let obj_seed = a.sys.hooks.objects.as_ref().map_or(0, |o| o.obj_seed);
    let skills = a.sys.hooks.tables.skills.skills.len();
    a.sys.hooks.session.names.insert(player, entry.name);
    // The player's own add messages, placed nowhere.
    a.sys
        .hooks
        .x
        .send(player, &assign_player(guid, class as u8, &entry.name, 0, 0));
    a.with(&mut s.game, |g, v| v.player_part_b(g, player, player));
    // The skills section's 0x94, from the player's list.
    if entry.base_skills {
        let m = a
            .sys
            .hooks
            .skill_lists
            .get(&player)
            .and_then(|l| msg::base_skill_levels(guid, &join_base_levels(l)));
        if let Some(m) = m {
            a.sys.hooks.x.send(player, &m);
        }
    }
    // The loader's item calls send S→C 0x22 (the item-skill link of a scroll
    // or tome) and 0x21 (the stat 97 / 107 callback), in the save's item
    // order, after 0x94 and before the 0x23; so does the direct 0x9D action 5
    // (flag 0x20) of a stored item the runeword refresh removes (sent inside
    // the loader). The other item messages (0x9C, 0x9D) are sent later, with
    // the player's item messages.
    let mut items = a.sys.hooks.session.join_items.remove(&player);
    if let Some(list) = items.as_mut() {
        let (loader, rest): (Vec<_>, Vec<_>) = std::mem::take(list)
            .into_iter()
            .partition(|m| matches!(m[..], [0x21..=0x23, ..] | [0x9D, 0x05, ..]));
        *list = rest;
        for m in &loader {
            a.sys.hooks.x.send(player, m);
        }
    }
    // The full save's post-load selections: left (hand 1), right. A
    // selection sends 0x23 only when the player's list holds the entry of
    // (skill, owner -1); a class skill with no entry sends nothing. Tested
    // here, after the loader's item callbacks (stat 97 / 107) have added
    // their entries.
    for (hand, sel) in [(1u8, entry.load_selects[0]), (0, entry.load_selects[1])] {
        let has = |k: u16| {
            a.sys
                .hooks
                .skill_lists
                .get(&player)
                .is_some_and(|l| l.find(i32::from(k), -1).is_some())
        };
        if let Some(skill) = sel.filter(|&k| has(k)) {
            a.sys
                .hooks
                .x
                .send(player, &msg::set_skill(0, guid, hand, skill, u32::MAX));
        }
        // An item-owned saved skill: the entry is found by skill and owner
        // GUID; it was selected when the hand it ends in holds that skill
        // with an owner item (the join's own pair of 0x23).
        let item_sel = entry.load_item_selects[if hand == 1 { 0 } else { 1 }];
        let h = entry.record.as_ref().map(|r| r.hands[usize::from(hand)]);
        if let (Some(skill), Some(h)) = (item_sel, h) {
            if h.skill == skill && h.item != u32::MAX {
                a.sys
                    .hooks
                    .x
                    .send(player, &msg::set_skill(0, guid, hand, skill, h.item));
            }
        }
    }
    // The stub load's right-skill selection.
    if let Some(h) = entry.load_skill {
        a.sys
            .hooks
            .x
            .send(player, &msg::set_skill(0, guid, 0, h.skill, h.item));
    }
    // The quest entry's messages (0x5E, 0x28, 0x29, 0x89).
    if let Some(quest) = a.sys.hooks.session.join_quest.remove(&player) {
        for m in &quest {
            a.sys.hooks.x.send(player, m);
        }
    }
    // Handshake, portal flags, stats, items, hot keys, skill hands.
    let x = &mut a.sys.hooks.x;
    x.send(player, &msg::unit_ref(0x0B, 0, guid));
    if let Some(r) = &entry.record {
        x.send(player, &msg::portal_flags(r.portal_flags));
    }
    // The stat messages (the mod array is cleared only by the room update
    // queue, so the second send below carries the same values).
    let stats = stat_messages(&a.sys.stats.mod_values(player));
    let x = &mut a.sys.hooks.x;
    for m in &stats {
        x.send(player, m);
    }
    // The player's item messages (the loader's, in send order), then
    // flag-ex bit 21: the first tick's per-client update runs the inventory
    // refresh (S→C 0x48).
    for m in items.iter().flatten() {
        x.send(player, m);
    }
    if items.is_some() {
        if let Some(r) = a.sys.units.get_mut(player) {
            r.flags2 |= d2_sim::wiring::action::INVENTORY_REFRESH_EX;
        }
    }
    let x = &mut a.sys.hooks.x;
    for (i, k) in entry.hotkeys.iter().enumerate() {
        if usize::try_from(k.skill).is_ok_and(|sk| sk < skills) {
            x.send(
                player,
                &msg::assign_hotkey(i as u8, k.skill, k.flag, k.item),
            );
        }
    }
    if let Some(r) = &entry.record {
        for hand in [1u8, 0] {
            let h = r.hands[usize::from(hand)];
            x.send(player, &msg::set_skill(0, guid, hand, h.skill, h.item));
        }
    }
    // The stat messages again.
    for m in &stats {
        x.send(player, m);
    }
    // The vitals sync.
    for m in vitals_sync::join_run(a, &s.game, id, (0, 0)) {
        a.sys.hooks.x.send(player, &m);
    }
    // The client's act slot is built when empty (only acts 1 and 2 are made
    // at game creation, so a save standing in Act III–V has none yet).
    s.game
        .lists
        .ensure_act(entry.act)
        .map_err(|_| JoinError::NoAct(entry.act))?;
    // The player's act byte is the client act from the join on.
    // The game entry's placement sets it from the spawn room's level;
    // nothing reads it in between, so writing the same value here is
    // equivalent.
    if let Some(r) = a.sys.units.get_mut(player) {
        r.act = entry.act;
    }
    a.sys
        .hooks
        .x
        .send(player, &load_act(entry.act, map_seed, obj_seed).encode());
    // The act is built here when its slot is empty: a fresh environment
    // record.
    let env = s
        .game
        .lists
        .act_mut(entry.act)
        .map(|r| {
            if !r.built {
                r.built = true;
                r.environment = d2_sim::world::environment::Environment::CREATED;
            }
            r.environment
        })
        .ok_or(JoinError::NoAct(entry.act))?;
    a.sys.hooks.x.send(player, &env.message());
    if let Some(e) = s.game.lists.client_mut(id) {
        e.state = client_state::ACT_LOADED;
    }
    // Game entry.
    let errors = a.sys.hooks.errors.len();
    let placed = a.with(&mut s.game, |g, v| {
        game_entry(PathCtx::of(v, g), player, entry.act)
    });
    if !placed {
        let why = a.sys.hooks.errors[errors..]
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(JoinError::Entry(why));
    }
    // The end of the game entry re-casts the loaded Iron Golem at the
    // placed player and clears the client's `golem_pending` flag.
    let golem = s
        .game
        .lists
        .client_mut(id)
        .map(|e| std::mem::take(&mut e.golem_pending))
        .unwrap_or(false);
    if golem {
        // The summon's monster init needs the lent world (as the
        // hireling restore's). No skill 90 entry or no spawn: nothing.
        let game = &mut s.game;
        let _ = s.events.lend_world(|a| a.golem_resummon(game, player));
        // The cast ends with the item equipped on the golem.
        let faults = s.world.place_golem_item(&mut s.game, &mut s.events, player);
        // A fault leaves the golem without its item; the load's report has
        // no line for it (D2RUST's own).
        let _ = faults;
    }
    // The join step after the player's allocation: neutral mode start, then
    // the regeneration event every frame (life, stamina, mana) and the
    // refresh event.
    let a = s.events.action();
    let joined = a.sys.with(&mut s.game, |sim, hooks| {
        d2_sim::units::modes::player_join(sim, hooks, player)
    });
    if let Err(e) = joined {
        return Err(JoinError::Entry(format!("{e:?}")));
    }
    // The passive states' unit bits go on after the join messages
    // (`skill_events::passive_states_on`), with the right aura's state. The
    // packet order is pinned by the skill packet tests; the exact call site
    // is provisional.
    s.events.action().passive_states_on(&mut s.game, player);
    // Client state 3.
    if let Some(e) = s.game.lists.client_mut(id) {
        e.state = client_state::JOINING;
    }
    Ok(player)
}

/// The join of `client` with its character save: the load of `save`
/// (parsed and checked by `d2_formats::d2s`) onto the client's player unit
/// on the action wiring ([`ActionCharacter`]), then, for a full save, the
/// caller's quest entry with mode 0, then the game entry ([`enter_game`])
/// in the act the save holds (a new character enters act 0). Steps without
/// a provider are listed in the report.
///
/// TODO: whether the game entry should also run the quest entry with mode 0
/// after a new-character start is undecided; it is not run here.
pub fn enter_game_from_save<D: ActionEvents, W: GolemWorld>(
    s: &mut SimGame<D, W>,
    client: ClientId,
    save: &D2s,
    ctx: &LoadContext,
) -> Result<(UnitId, LoadReport), JoinError> {
    if s.sim_client(client).is_none() {
        return Err(JoinError::NotJoined(client));
    }
    let player = s.player_of(client).ok_or(JoinError::NoPlayer(client))?;
    let (entry, report) = load_save(s, player, save, ctx)?;
    let placed = enter_game(s, client, &entry)?;
    Ok((placed, report))
}

/// The load part of [`enter_game_from_save`] on `player` (no room yet):
/// the load of `save` on the action wiring, then, for a full save, the
/// quest entry with mode 0; returns the join's [`Entry`] (the save's act
/// and name). A session flow's character loader calls it
/// (`super::session_flow::CharacterLoader`).
pub fn load_save<D: ActionEvents, W>(
    s: &mut SimGame<D, W>,
    player: UnitId,
    save: &D2s,
    ctx: &LoadContext,
) -> Result<(Entry, LoadReport), LoadError> {
    let report = s.events.action().with(&mut s.game, |_, v| {
        let mut cw = ActionCharacter { v, player };
        let mut r = character::load(save, ctx, &mut cw)?;
        if !r.new_character {
            if let Err(u) = cw.quest_entry(0) {
                r.unapplied.push(u);
            }
        }
        Ok::<_, LoadError>(r)
    })?;
    let entry = if report.new_character {
        let portals = s.events.action().sys.hooks.drlg.data.portal_levels();
        Entry::new_character(save.header.name, &portals, report.right_skill)
    } else {
        Entry::loaded(
            report.act,
            save.header.name,
            report.mouse_selected,
            report.mouse_item_selected,
        )
    };
    Ok((entry, report))
}

/// A new character's load on `player` (the stub path) and its join values
/// ([`Entry::new_character`]).
pub fn load_new_character<D: ActionEvents, W>(
    s: &mut SimGame<D, W>,
    player: UnitId,
    name: [u8; 16],
) -> (Entry, LoadReport) {
    let a = s.events.action();
    let report = a.with(&mut s.game, |_, v| {
        character::load_new_character(&mut ActionCharacter { v, player })
    });
    let portals = a.sys.hooks.drlg.data.portal_levels();
    (
        Entry::new_character(name, &portals, report.right_skill),
        report,
    )
}

/// [`load_new_character`] on the wired host, with the start items
/// made on its economy and inventory model
/// ([`WiredWorld::start_items`]) in place of the action wiring's
/// unapplied step. Start stats and start items are the load's only steps
/// before the start-skill selection that touch the player, and neither
/// that selection nor the mouse skills draw, so running the items after
/// the action wiring's load keeps the game-seed draw order. The
/// "start items" entry leaves the report's unapplied list when the items
/// ran (no fault).
pub fn load_new_character_with_items<D: ActionEvents, R, S>(
    s: &mut SimGame<D, WiredWorld<R, S>>,
    player: UnitId,
    name: [u8; 16],
) -> (Entry, LoadReport, StartItems) {
    let (entry, mut report) = load_new_character(s, player, name);
    let items = s.world.start_items(&mut s.game, &mut s.events, player);
    if items.faults.is_empty() {
        report.unapplied.retain(|u| u.step != "start items");
    }
    (entry, report, items)
}

/// The native entries the join's S→C 0x94 lists: the skills section's
/// levels. A base-0 entry is one an item's stat 97 / 107 callback added at
/// the load; that callback runs after the 0x94, so it reaches the client as
/// the loader's 0x21 instead.
pub(crate) fn join_base_levels(l: &d2_sim::skills::list::SkillList) -> Vec<(u16, u8)> {
    l.base_levels()
        .into_iter()
        .filter(|&(_, b)| b != 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portal level list.
    const PORTALS: [u32; 16] = [1, 3, 5, 7, 27, 29, 33, 36, 40, 43, 45, 46, 53, 54, 74, 134];

    #[test]
    fn stat_message_size_follows_the_unsigned_value() {
        assert_eq!(stat_message(12, 1), Some(vec![0x1D, 12, 1]));
        assert_eq!(stat_message(0, 0xFE), Some(vec![0x1D, 0, 0xFE]));
        // 0xFF goes to 0x1E, 0xFFFF to 0x1F.
        assert_eq!(stat_message(7, 0xFF), Some(vec![0x1E, 7, 0xFF, 0]));
        assert_eq!(stat_message(7, 0xFFFE), Some(vec![0x1E, 7, 0xFE, 0xFF]));
        assert_eq!(
            stat_message(7, 0xFFFF),
            Some(vec![0x1F, 7, 0xFF, 0xFF, 0, 0])
        );
        // A negative value is a large unsigned one.
        assert_eq!(
            stat_message(15, -1),
            Some(vec![0x1F, 15, 0xFF, 0xFF, 0xFF, 0xFF])
        );
        // s > 0xFE cannot be sent: nothing.
        assert_eq!(stat_message(0xFE, 0), Some(vec![0x1D, 0xFE, 0]));
        assert_eq!(stat_message(0xFF, 0), None);
        assert_eq!(stat_message(300, 0), None);
    }

    #[test]
    fn stat_messages_drop_the_layer_and_keep_key_order() {
        use d2_sim::stats::key;
        let v = [(key(0, 0), 15), (key(7, 0), 0x2800), (key(12, 3), 1)];
        assert_eq!(
            stat_messages(&v),
            vec![
                vec![0x1D, 0, 15],
                vec![0x1E, 7, 0x00, 0x28],
                vec![0x1D, 12, 1],
            ]
        );
        assert!(stat_messages(&[]).is_empty());
    }

    #[test]
    fn portal_flags_are_the_bit_of_level_1() {
        assert_eq!(initial_portal_flags(&PORTALS), 1);
        assert_eq!(initial_portal_flags(&[3, 5, 1]), 4);
        // Level 1 absent → 0.
        assert_eq!(initial_portal_flags(&[3, 5]), 0);
        assert_eq!(initial_portal_flags(&[]), 0);
    }

    #[test]
    fn a_new_character_has_its_record_and_load_skill() {
        let e = Entry::new_character(*b"Sorc\0\0\0\0\0\0\0\0\0\0\0\0", &PORTALS, Some(36));
        assert_eq!(e.act, 0);
        assert_eq!(e.hotkeys, [NO_HOT_KEY; 16]);
        let r = e.record.unwrap();
        assert_eq!(r.portal_flags, 1);
        // Hand 0 = `StartSkill`, hand 1 = 0; both items 0. The load's own
        // 0x23 keeps the class skill's owner −1.
        assert_eq!(r.hands[0], SkillHand { skill: 36, item: 0 });
        assert_eq!(r.hands[1], SkillHand { skill: 0, item: 0 });
        assert_eq!(
            e.load_skill,
            Some(SkillHand {
                skill: 36,
                item: u32::MAX
            })
        );
        // No start skill: hand 0 is 0 and the load sends no 0x23.
        let e = Entry::new_character([0; 16], &PORTALS, None);
        assert_eq!(e.record.unwrap().hands[0], SkillHand { skill: 0, item: 0 });
        assert_eq!(e.load_skill, None);
    }
}
