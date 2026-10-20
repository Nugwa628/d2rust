//! Character storage: what loading a parsed `.d2s` does to the game.
//! `d2_formats::d2s` parses and checks the bytes; [`load`] then runs either the
//! new-character start or the load effects of a full save, step by step in a
//! fixed order, each step on its owner's seam ([`CharacterWorld`]). The session
//! layer calls it when a player joins, and [`save`] is the writer that builds
//! the file back from the game.
//!
//! What is decided here: the header's client fields (act from the town byte,
//! the map seed condition), the hotkey and mouse-skill decode, the quest record
//! normalisation, the stat entries, the item flags of a loaded record
//! ([`d2_formats::d2s::item_flags_on_load`]), the golem's skill 90 check and the
//! post-load values (gold limits, stamina, hitpoints and mana, stats 67-69 and
//! 30).
//!
//! The player unit exists before the load here (the session joins it,
//! `session::enter_game`); in the game the header step allocates it. The game
//! entry after a successful load (the quest entry with mode 0 and the
//! messages) is the caller's (`session::enter_game_from_save`).

use d2_formats::d2s::{self, Corpse, D2s, Header, Hireling, ItemEntry, Slot, StatEntry};
use d2_sim::combat::vitals::{init_player_stats, VitalsUnits};
use d2_sim::skills::list::{class_skills, ListOwner};
use d2_sim::units::UnitId;
use d2_sim::wiring::action::{HirelingCall, Pending, View};
use d2_sim::world::hirelings::life::{Loader, SavedHireling};
use d2_sim::world::waypoints::{WaypointRecord, WaypointRecords};

/// Stat ids the load reads or writes.
pub mod stat {
    /// Current hitpoints.
    pub const HITPOINTS: u16 = 6;
    /// Current mana.
    pub const MANA: u16 = 8;
    /// Current stamina.
    pub const STAMINA: u16 = 10;
    /// Maximum stamina.
    pub const MAXSTAMINA: u16 = 11;
    /// Character level.
    pub const LEVEL: u16 = 12;
    /// Gold carried.
    pub const GOLD: u16 = 14;
    /// Gold in the stash.
    pub const GOLDBANK: u16 = 15;
    /// Experience needed for the next level.
    pub const NEXTEXP: u16 = 30;
    /// Velocity percent (reset to 100 after a load).
    pub const VELOCITYPERCENT: u16 = 67;
    /// Attack rate percent (reset to 100 after a load).
    pub const ATTACKRATE: u16 = 68;
    /// Other animation rate percent (reset to 100 after a load).
    pub const OTHER_ANIMRATE: u16 = 69;
}

/// Iron Golem skill: a saved golem item needs the player to have it.
pub const SKILL_IRON_GOLEM: u16 = 90;
/// Gold carry limit per character level.
pub const GOLD_PER_LEVEL: i32 = 10_000;
/// Stash gold limit.
pub const GOLDBANK_MAX: i32 = 2_500_000;
/// Quest slots the normalisation covers (42 per difficulty record).
pub const QUEST_SLOTS: usize = 42;

/// What the game knows at load time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadContext {
    /// The game's difficulty (0 Normal, 1 Nightmare, 2 Hell).
    pub difficulty: u8,
    /// The save's map seed applies (a game of type 3; every D2RUST caller
    /// passes false today): a town byte with 0x80 then sets the game's map
    /// seed to the save's.
    pub map_seed_applies: bool,
}

/// One decoded hotkey or mouse skill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillSlot {
    /// −1: no skill.
    pub skill: i32,
    /// The skill goes on the left button.
    pub left: bool,
    /// 1-based inventory position; −1: none.
    pub item: i32,
}

impl From<Slot> for SkillSlot {
    fn from(s: Slot) -> Self {
        let (skill, left, item) = s.decode();
        SkillSlot { skill, left, item }
    }
}

/// The client and player fields the header step sets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeaderLoad {
    /// Client weapon switch := the header's `weapon_switch` & 1.
    pub weapon_switch: bool,
    /// Client status := the header's `status`.
    pub status: u16,
    /// Client create time := the header's `create_time` (the load is the only place that sets it).
    pub create_time: u32,
    /// Client act := (the difficulty's town byte) & 0x7F, 0 when ≥ 5.
    pub act: u8,
    /// The game's map seed := the header's `map_seed` when the town byte has 0x80 and
    /// [`LoadContext::map_seed_applies`].
    pub map_seed: Option<u32>,
    /// Hotkeys handed to the client.
    pub hotkeys: [SkillSlot; 16],
    /// Left, right, swap left, swap right.
    pub mouse: [SkillSlot; 4],
    /// The client byte set from the header's `client_cf` (the two bytes
    /// after it := 0).
    pub client_480: u8,
}

/// The header's client fields. The town byte `t` is the header's
/// `towns` entry of the difficulty (one byte per difficulty); its low 7 bits are the act (0 when 5 or more) and
/// bit 0x80 requests the map seed.
pub fn header_load(h: &Header, ctx: &LoadContext) -> HeaderLoad {
    let t = h
        .towns
        .get(usize::from(ctx.difficulty))
        .copied()
        .unwrap_or(0);
    let act = t & 0x7F;
    HeaderLoad {
        weapon_switch: h.weapon_switch & 1 != 0,
        status: h.status,
        create_time: h.create_time,
        act: if act >= 5 { 0 } else { act },
        map_seed: (t & 0x80 != 0 && ctx.map_seed_applies).then_some(h.map_seed),
        hotkeys: h.hotkeys.map(SkillSlot::from),
        mouse: h.mouse.map(SkillSlot::from),
        client_480: h.client_cf,
    }
}

/// Normalises a quest record on load: for every
/// slot q = 0..41 (u16 at 2q), clear bits 13 and 14, then bit 1 set →
/// set bit 15. Bytes past the 42 slots are kept.
pub fn normalise_quests(rec: &[u8; 96]) -> [u8; 96] {
    let mut out = *rec;
    for q in 0..QUEST_SLOTS {
        let at = 2 * q;
        let mut v = u16::from_le_bytes([out[at], out[at + 1]]);
        v &= !((1 << 13) | (1 << 14));
        if v & (1 << 1) != 0 {
            v |= 1 << 15;
        }
        out[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// Gold limits applied after a load: gold < 0 or above level × 10,000 → 0; goldbank
/// < 0 or above 2,500,000 → 0.
pub fn gold_limits(gold: i32, goldbank: i32, level: i32) -> (i32, i32) {
    let carry = level.wrapping_mul(GOLD_PER_LEVEL);
    let g = if gold < 0 || gold > carry { 0 } else { gold };
    let b = if !(0..=GOLDBANK_MAX).contains(&goldbank) {
        0
    } else {
        goldbank
    };
    (g, b)
}

/// The item lists of a save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemList {
    /// The player's own items.
    Player,
    /// The items on a corpse.
    Corpse,
    /// The hireling's items.
    Hireling,
    /// The golem's item.
    Golem,
}

/// A load step (of a full save or of the new-character start) that the
/// provider could not apply, with its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unapplied {
    /// The load step that was not applied.
    pub step: &'static str,
    /// Why no provider could apply it.
    pub reason: &'static str,
}

/// The seams of the load steps, each on the system that owns the step. Each
/// step returns `Err(Unapplied)` when its provider does not exist yet (named,
/// never guessed); [`load`] records that and goes on, in the fixed load order.
pub trait CharacterWorld {
    /// The player init's native skills: the player's skill list gets
    /// skill 0 and its class's `charstats` `Skill 1`…`Skill 10` (base 1,
    /// owner −1), with Attack in both hands. Both load paths allocate the
    /// player through that init (the new-character start, and the header
    /// step of a full save), so it runs first in both.
    fn player_skills(&mut self) -> Result<(), Unapplied>;

    // ---- new-character start

    /// The client's first state bit set, the unit's set-up (mode 1, four
    /// player-data words := 0) and its add messages.
    fn new_character_setup(&mut self) -> Result<(), Unapplied>;
    /// Start stats for the client act (`init_player_stats`).
    fn start_stats(&mut self, act: u8) -> Result<(), Unapplied>;
    /// Start items: the class's `charstats` item slots (`start_items`).
    fn start_items(&mut self) -> Result<(), Unapplied>;
    /// The class's `charstats` `StartSkill`; 0: none.
    fn start_skill(&self) -> Result<u16, Unapplied>;
    /// The player has the skill in its skill list.
    fn has_skill(&self, skill: u16) -> Result<bool, Unapplied>;
    /// The right and left mouse skills := 0, then the right skill
    /// selected with no item (item −1) when given.
    fn set_mouse_skills(&mut self, right: Option<u16>) -> Result<(), Unapplied>;
    /// The quest entry of the player with `mode`.
    fn quest_entry(&mut self, mode: u8) -> Result<(), Unapplied>;

    // ---- a full save

    /// The header's client and player fields.
    fn apply_header(&mut self, h: &HeaderLoad) -> Result<(), Unapplied>;
    /// The three quest records, normalised ([`normalise_quests`]).
    fn set_quests(&mut self, records: &[[u8; 96]; 3]) -> Result<(), Unapplied>;
    /// The waypoint records (16 bytes per difficulty).
    fn set_waypoints(&mut self, records: &[[u8; 16]; 3]) -> Result<(), Unapplied>;
    /// The NPC fields A and B per difficulty.
    fn set_npc_fields(&mut self, a: &[[u8; 8]; 3], b: &[[u8; 8]; 3]) -> Result<(), Unapplied>;
    /// One base stat.
    fn set_base_stat(&mut self, id: u16, layer: u16, value: i32) -> Result<(), Unapplied>;
    /// The total of a stat (layer 0).
    fn stat(&self, id: u16) -> Result<i32, Unapplied>;
    /// The base level of the class
    /// skill at list index `index` added.
    fn add_skill_level(&mut self, index: usize, level: u8) -> Result<(), Unapplied>;
    /// The list's entries created (flags of [`d2s::item_flags_on_load`],
    /// replenish timers), placed and their children inserted (the golem
    /// item gets mode 3 and goes to the client).
    fn create_items(&mut self, list: ItemList, entries: &[ItemEntry]) -> Result<(), Unapplied>;
    /// The corpse unit, linked to the player (its
    /// items go through [`CharacterWorld::create_items`] after it).
    fn create_corpse(&mut self, corpse: &Corpse) -> Result<(), Unapplied>;
    /// Restores the saved hireling (nothing when the block holds none).
    fn restore_hireling(&mut self, block: &Hireling) -> Result<(), Unapplied>;
    /// Runs after the hireling's items are created.
    fn hireling_items_loaded(&mut self) -> Result<(), Unapplied>;
    /// Post-load: item indices → GUIDs and
    /// the left and right skills selected with their item.
    fn resolve_item_indices(&mut self) -> Result<(), Unapplied>;
    /// The experience of the next level for the player's class.
    fn next_exp(&self, level: i32) -> Result<i32, Unapplied>;
}

/// Why a load failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// A golem item without a skill 90 entry (internal code 23).
    #[error("golem item without the Iron Golem skill (internal code 23)")]
    GolemSkill,
}

impl LoadError {
    /// The load result code the client is told for the error (see
    /// [`refusal_message`]).
    pub fn result(&self) -> u32 {
        match self {
            // Internal 23 → 10.
            LoadError::GolemSkill => 10,
        }
    }
}

/// What the load did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// The new-character start ran (the 335-byte stub).
    pub new_character: bool,
    /// The client act (game entry's act).
    pub act: u8,
    /// Steps the provider could not apply, in order.
    pub unapplied: Vec<Unapplied>,
    /// New-character start: the right skill selected (`StartSkill`), if any.
    pub right_skill: Option<u16>,
    /// A full save: the left and right class skills the post-load
    /// selection sends
    /// S→C 0x23 for (owner −1: no item; an item-owned selection needs the
    /// item list, which the provider lacks).
    pub mouse_selected: [Option<u16>; 2],
    /// The same for a saved mouse skill owned by an item (item index > 0):
    /// the entry is found by skill and owner GUID,
    /// the GUID resolved by the join from the hand it ends in.
    pub mouse_item_selected: [Option<u16>; 2],
}

fn note(r: &mut LoadReport, res: Result<(), Unapplied>) {
    if let Err(u) = res {
        r.unapplied.push(u);
    }
}

/// Runs the load of `save` (parsed and checked by `d2_formats::d2s`) on
/// `w`: the new-character start for a stub, the full-save load otherwise.
pub fn load(
    save: &D2s,
    ctx: &LoadContext,
    w: &mut dyn CharacterWorld,
) -> Result<LoadReport, LoadError> {
    let mut r = LoadReport::default();
    let Some(body) = &save.body else {
        // The stub: the client act stays 0 (no header step runs), so
        // `StartSkill` applies.
        return Ok(load_new_character(w));
    };
    // The header step allocates the player, which runs the player init.
    note(&mut r, w.player_skills());
    let h = header_load(&save.header, ctx);
    r.act = h.act;
    // Left first, then right; skill 0 (Attack) is
    // never selected there. A decoded "no item" (-1) is a class entry.
    r.mouse_selected =
        [h.mouse[0], h.mouse[1]].map(|m| (m.skill > 0 && m.item == -1).then_some(m.skill as u16));
    r.mouse_item_selected =
        [h.mouse[0], h.mouse[1]].map(|m| (m.skill > 0 && m.item > 0).then_some(m.skill as u16));
    note(&mut r, w.apply_header(&h));
    let quests = body.quests.records.map(|q| normalise_quests(&q));
    note(&mut r, w.set_quests(&quests));
    note(&mut r, w.set_waypoints(&body.waypoints.records));
    note(&mut r, w.set_npc_fields(&body.npcs.a, &body.npcs.b));
    // Stats, then hitpoints and mana remembered for the post-load step.
    for StatEntry { id, layer, value } in body.stats.entries() {
        note(&mut r, w.set_base_stat(id, layer, value));
    }
    let remembered = (w.stat(stat::HITPOINTS), w.stat(stat::MANA));
    // Skills: 0x2A bytes, one base level each.
    for (i, &lvl) in body.skills.iter().enumerate() {
        if lvl != 0 {
            note(&mut r, w.add_skill_level(i, lvl));
        }
    }
    note(&mut r, w.create_items(ItemList::Player, &body.items));
    for c in &body.corpses {
        note(&mut r, w.create_corpse(c));
        note(&mut r, w.create_items(ItemList::Corpse, &c.items));
    }
    // Hireling (classic and expansion alike; the restore itself answers
    // "no hireling"), then its items (read only for a restored hireling).
    note(&mut r, w.restore_hireling(&save.header.hireling));
    if let Some(Some(items)) = &body.hireling_items {
        note(&mut r, w.create_items(ItemList::Hireling, items));
        note(&mut r, w.hireling_items_loaded());
    }
    // Golem: its item needs the Iron Golem skill.
    if let Some(g) = &body.golem {
        if let Some(item) = &g.item {
            match w.has_skill(SKILL_IRON_GOLEM) {
                Ok(true) => note(
                    &mut r,
                    w.create_items(ItemList::Golem, std::slice::from_ref(item)),
                ),
                Ok(false) => return Err(LoadError::GolemSkill),
                Err(u) => r.unapplied.push(u),
            }
        }
    }
    post_load(w, remembered, &mut r);
    Ok(r)
}

/// The new-character start on `w` (the stub's load, and a new character): the
/// client act stays 0 (no header step runs), so `StartSkill` applies.
pub fn load_new_character(w: &mut dyn CharacterWorld) -> LoadReport {
    let mut r = LoadReport {
        new_character: true,
        act: 0,
        ..LoadReport::default()
    };
    r.right_skill = new_character(w, r.act, &mut r);
    r
}

/// The new-character start after the unit exists; returns the right skill selected.
fn new_character(w: &mut dyn CharacterWorld, act: u8, r: &mut LoadReport) -> Option<u16> {
    // Allocating the player runs the player init.
    note(r, w.player_skills());
    note(r, w.new_character_setup());
    note(r, w.start_stats(act));
    note(r, w.start_items());
    let right = if act == 0 {
        match w.start_skill() {
            // An empty `StartSkill` reads as -1 (0xFFFF): no start skill.
            Ok(0) | Ok(u16::MAX) => None,
            Ok(s) => match w.has_skill(s) {
                Ok(true) => Some(s),
                Ok(false) => None,
                Err(u) => {
                    // With no provider for the entries the start items
                    // grant (stat 107 on slot 0, see `start_items`) the
                    // start skill is taken as present, as a fresh
                    // character plays (Sorceress 36, Necromancer 70 =
                    // `StartSkill`: S→C 0x23 for hand 0 with that skill).
                    r.unapplied.push(u);
                    Some(s)
                }
            },
            Err(u) => {
                r.unapplied.push(u);
                None
            }
        }
    } else {
        None
    };
    note(r, w.set_mouse_skills(right));
    note(r, w.quest_entry(1));
    right
}

/// The post-load values: gold limits, stamina, item indices,
/// hitpoints and mana, stats 67–69, then stat 30.
fn post_load(
    w: &mut dyn CharacterWorld,
    remembered: (Result<i32, Unapplied>, Result<i32, Unapplied>),
    r: &mut LoadReport,
) {
    let read = |w: &dyn CharacterWorld, id, r: &mut LoadReport| match w.stat(id) {
        Ok(v) => Some(v),
        Err(u) => {
            r.unapplied.push(u);
            None
        }
    };
    let gold = read(w, stat::GOLD, r);
    let bank = read(w, stat::GOLDBANK, r);
    let level = read(w, stat::LEVEL, r);
    if let (Some(g), Some(b), Some(l)) = (gold, bank, level) {
        let (g2, b2) = gold_limits(g, b, l);
        if g2 != g {
            note(r, w.set_base_stat(stat::GOLD, 0, g2));
        }
        if b2 != b {
            note(r, w.set_base_stat(stat::GOLDBANK, 0, b2));
        }
    }
    if let Some(max) = read(w, stat::MAXSTAMINA, r) {
        note(r, w.set_base_stat(stat::STAMINA, 0, max));
    }
    note(r, w.resolve_item_indices());
    for (id, v) in [(stat::HITPOINTS, remembered.0), (stat::MANA, remembered.1)] {
        match v {
            Ok(v) => note(r, w.set_base_stat(id, 0, v)),
            Err(u) => r.unapplied.push(u),
        }
    }
    for id in [
        stat::VELOCITYPERCENT,
        stat::ATTACKRATE,
        stat::OTHER_ANIMRATE,
    ] {
        note(r, w.set_base_stat(id, 0, 100));
    }
    if let Some(l) = level {
        match w.next_exp(l) {
            Ok(n) => note(r, w.set_base_stat(stat::NEXTEXP, 0, n)),
            Err(u) => r.unapplied.push(u),
        }
    }
}

/// The load steps on the action wiring's unit side (`d2_sim::wiring::action::View`):
/// the player's stat list (set, totals, `maxstamina`), the start stats
/// (`init_player_stats`), `StartSkill` and the
/// experience table (`VitalsTables`). Every other step has no provider in
/// `d2-sim` yet and is reported as [`Unapplied`] with its owner.
pub struct ActionCharacter<'v, 'a, X> {
    /// The action wiring's unit side.
    pub v: &'v mut View<'a, X>,
    /// The player being loaded.
    pub player: UnitId,
}

fn unapplied<T>(step: &'static str, reason: &'static str) -> Result<T, Unapplied> {
    Err(Unapplied { step, reason })
}

impl<X: Pending> ActionCharacter<'_, '_, X> {
    fn class(&self) -> i32 {
        self.v.units.get(self.player).map_or(0, |r| r.class as i32)
    }
}

impl<X: Pending> CharacterWorld for ActionCharacter<'_, '_, X> {
    /// The player init's skills on the player's list in `ActionHooks::skill_lists`
    /// (created here when missing).
    fn player_skills(&mut self) -> Result<(), Unapplied> {
        let Some(t) = self.v.h.vitals.clone() else {
            return unapplied(
                "player skills",
                "no vitals tables (charstats `Skill 1`–`Skill 10`) on the action wiring",
            );
        };
        let class = self.class();
        let cs = t.charstats(class).map(|c| {
            [
                c.skill_1, c.skill_2, c.skill_3, c.skill_4, c.skill_5, c.skill_6, c.skill_7,
                c.skill_8, c.skill_9, c.skill_10,
            ]
        });
        let h = &mut *self.v.h;
        let rows = &h.tables.skills.skills;
        let list = h.skill_lists.entry(self.player).or_default();
        list.init_player(rows, ListOwner::player(class), cs.as_ref())
            .map_err(|_| Unapplied {
                step: "player skills",
                reason: "select of skill 0 outside the skills table (fatal 0x668)",
            })
    }
    fn new_character_setup(&mut self) -> Result<(), Unapplied> {
        unapplied(
            "new character set-up",
            "new character mode, player data and add messages: player data and part B of the add messages are not in d2-sim (intents-events.md §7.2)",
        )
    }
    fn start_stats(&mut self, act: u8) -> Result<(), Unapplied> {
        let Some(t) = self.v.h.vitals.clone() else {
            return unapplied("start stats", "no vitals tables on the action wiring");
        };
        init_player_stats(&mut *self.v, &t, self.player, u32::from(act));
        Ok(())
    }
    fn start_items(&mut self) -> Result<(), Unapplied> {
        unapplied(
            "start items",
            "start item generation (items/generation.md §10.3) has no d2-sim caller on a player unit",
        )
    }
    fn start_skill(&self) -> Result<u16, Unapplied> {
        let Some(t) = self.v.h.vitals.as_ref() else {
            return unapplied("start skill", "no vitals tables on the action wiring");
        };
        let s = t.charstats(self.class()).map_or(0, |c| c.startskill);
        // A blank `StartSkill` (the Amazon's, Barbarian's, ... read as
        // 0xFFFF) names no `skills` row, so the list has no entry for it
        // and no right skill is selected.
        if usize::from(s) >= self.v.h.tables.skills.skills.len() {
            return Ok(0);
        }
        Ok(s)
    }
    /// An entry on the player's list → true. No entry is
    /// not an answer: the entries items grant (the start items' stat 107,
    /// step 2.3 of `start_items`; a loaded item's skills) are
    /// not in the list.
    fn has_skill(&self, skill: u16) -> Result<bool, Unapplied> {
        if self
            .v
            .h
            .skill_lists
            .get(&self.player)
            .is_some_and(|l| l.has(i32::from(skill)))
        {
            return Ok(true);
        }
        unapplied(
            "has skill",
            "item-granted skill entries (beyond the native list) have no provider here",
        )
    }
    fn set_mouse_skills(&mut self, _: Option<u16>) -> Result<(), Unapplied> {
        unapplied(
            "mouse skills",
            "mouse skill selection and player skill data are not in d2-sim",
        )
    }
    fn quest_entry(&mut self, _: u8) -> Result<(), Unapplied> {
        unapplied(
            "quest entry",
            "quest entry runs on the host's quest control (WiredWorld), not the action wiring",
        )
    }
    fn apply_header(&mut self, _: &HeaderLoad) -> Result<(), Unapplied> {
        unapplied(
            "header",
            "client record fields (+0x1AC act, status, hotkeys, +0x480) have no home in d2-sim",
        )
    }
    fn set_quests(&mut self, _: &[[u8; 96]; 3]) -> Result<(), Unapplied> {
        unapplied(
            "quests",
            "the player's quest records live in the host rest (QuestRest::quests)",
        )
    }
    /// The waypoint records, magic
    /// normalised, in `ActionHooks::waypoints`.
    fn set_waypoints(&mut self, records: &[[u8; 16]; 3]) -> Result<(), Unapplied> {
        let mut out = WaypointRecords::default();
        for (slot, src) in out.0.iter_mut().zip(records) {
            *slot = WaypointRecord::load_copy(src).map_err(|_| Unapplied {
                step: "waypoints",
                reason: "a record's magic is not 0x0102 / 0x0101 / 0 (waypoints.md §2)",
            })?;
        }
        self.v.h.waypoints.insert(self.player, out);
        Ok(())
    }
    fn set_npc_fields(&mut self, _: &[[u8; 8]; 3], _: &[[u8; 8]; 3]) -> Result<(), Unapplied> {
        unapplied(
            "npc fields",
            "the NPC record (player data +0x60) is the host's PlayerQuests: the host applies it",
        )
    }
    fn set_base_stat(&mut self, id: u16, layer: u16, value: i32) -> Result<(), Unapplied> {
        let v = &mut *self.v;
        v.stats.unit_set(&mut *v.h, self.player, id, value, layer);
        Ok(())
    }
    fn stat(&self, id: u16) -> Result<i32, Unapplied> {
        if id == stat::MAXSTAMINA {
            return Ok(VitalsUnits::max_stamina(&*self.v, self.player));
        }
        Ok(self.v.stats.unit_total(self.player, id, 0))
    }
    /// Skill = the class list's entry
    /// `index` (−1 past its end, which the set
    /// ignores), base level := `level`
    /// ([`SkillList::set_base`](d2_sim::skills::list::SkillList::set_base)).
    fn add_skill_level(&mut self, index: usize, level: u8) -> Result<(), Unapplied> {
        let class = self.class();
        let h = &mut *self.v.h;
        let rows = &h.tables.skills.skills;
        let Some(&skill) = class_skills(rows, class).get(index) else {
            return Ok(());
        };
        h.skill_lists.entry(self.player).or_default().set_base(
            rows,
            ListOwner::player(class),
            skill,
            level,
        );
        Ok(())
    }
    fn create_items(&mut self, _: ItemList, entries: &[ItemEntry]) -> Result<(), Unapplied> {
        if entries.is_empty() {
            return Ok(());
        }
        unapplied("items", "item units from save records have no d2-sim body")
    }
    fn create_corpse(&mut self, _: &Corpse) -> Result<(), Unapplied> {
        unapplied("corpse", "the corpse unit has no d2-sim body")
    }
    /// Queued for the host that holds the hireling lists
    /// (`ActionHooks::hireling_calls`, `HirelingCall::Restore`; the
    /// wired host restores the hireling from it, with a roomless
    /// allocation). The parser takes versions ≥ 0x5C only, so the
    /// loader is always the ≥ 0x5C one (`Loader::Current`).
    fn restore_hireling(&mut self, block: &Hireling) -> Result<(), Unapplied> {
        if !block.is_present() {
            // No hireling in the block: nothing to restore.
            return Ok(());
        }
        let Some(q) = self.v.h.hireling_calls.as_mut() else {
            return unapplied(
                "hireling",
                "no host holds the hireling lists (`ActionHooks::hireling_calls` is off)",
            );
        };
        q.push(HirelingCall::Restore {
            player: self.player,
            saved: SavedHireling {
                dead: block.flags & Hireling::DEAD != 0,
                seed: block.seed,
                name_index: block.name_index,
                id: block.id,
                experience: block.experience,
            },
            loader: Loader::Current,
        });
        Ok(())
    }
    fn hireling_items_loaded(&mut self) -> Result<(), Unapplied> {
        unapplied(
            "hireling rule 8",
            "the hireling's items are not created (see the items step), so rule 8 does not run",
        )
    }
    fn resolve_item_indices(&mut self) -> Result<(), Unapplied> {
        unapplied(
            "item indices",
            "hotkey loading needs the hotkeys and the player's inventory list",
        )
    }
    fn next_exp(&self, level: i32) -> Result<i32, Unapplied> {
        let Some(t) = self.v.h.vitals.as_ref() else {
            return unapplied("nextexp", "no vitals tables on the action wiring");
        };
        Ok(t.threshold(self.class(), level as u32) as i32)
    }
}

/// The flags each record of a loaded item entry gets, for
/// a provider that creates items from records.
pub fn loaded_flags(stored: u32) -> u32 {
    d2s::item_flags_on_load(stored)
}

/// One charstats start-item slot (`itemN` code, `itemNloc`, `itemNcount`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartSlot {
    /// The item code (`itemN`).
    pub code: [u8; 4],
    /// A `bodylocs` row; 0: the inventory.
    pub loc: u8,
    /// Copies to create (`itemNcount`).
    pub count: u8,
}

/// The ten start-item slots of a charstats row, in slot order.
pub fn start_slots(cs: &d2_data::tables::Charstats) -> [StartSlot; 10] {
    let s = |code, loc, count| StartSlot { code, loc, count };
    [
        s(cs.item1, cs.item1loc, cs.item1count),
        s(cs.item2, cs.item2loc, cs.item2count),
        s(cs.item3, cs.item3loc, cs.item3count),
        s(cs.item4, cs.item4loc, cs.item4count),
        s(cs.item5, cs.item5loc, cs.item5count),
        s(cs.item6, cs.item6loc, cs.item6count),
        s(cs.item7, cs.item7loc, cs.item7count),
        s(cs.item8, cs.item8loc, cs.item8count),
        s(cs.item9, cs.item9loc, cs.item9count),
        s(cs.item10, cs.item10loc, cs.item10count),
    ]
}

/// Quantity a quiver gets after its placement in the inventory.
pub const QUIVER_STORED: i32 = 100;
/// Quantity a quiver gets after its equip.
pub const QUIVER_EQUIPPED: i32 = 250;

/// Where a start item went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartPlace {
    /// Placed in the belt.
    Belt,
    /// Placed in the inventory.
    Inventory,
    /// Equipped at this body location.
    Equipped(u8),
    /// Every placement refused it: it stays with no place.
    Nowhere,
}

/// The seams of the start items on one player (`start_items`).
pub trait StartItemWorld {
    /// Code lookup and creation (source the player, spawn mode
    /// 4, quality 2, no sockets, never ethereal, ilvl the player's base
    /// level, no seeds). `None`: code not found or creation refused.
    fn create(&mut self, code: [u8; 4]) -> Option<UnitId>;
    /// Step 2.2: remove and free the item's flag-0x40 stat list, if any.
    fn drop_class_skill_list(&mut self, item: UnitId);
    /// Step 2.3: stat 107 layer `skill` := 1 in the flag-0x40 list
    /// (created if missing).
    fn set_single_skill(&mut self, item: UnitId, skill: u16);
    /// Whether the item is stackable.
    fn stackable(&mut self, item: UnitId) -> bool;
    /// Step 2.4: stat 70 := the total max stack.
    fn fill_stack(&mut self, item: UnitId);
    /// Step 2.5: item flag 0x20000; body location := `loc`.
    fn mark_start(&mut self, item: UnitId, loc: u8);
    /// Whether the item goes to the belt.
    fn beltable(&mut self, item: UnitId) -> bool;
    /// Belt placement (slot = item x, find 1).
    fn place_belt(&mut self, item: UnitId) -> bool;
    /// Inventory placement (page 0, find free, send 1).
    fn place_inventory(&mut self, item: UnitId) -> bool;
    /// Equip at `loc` (skip 1).
    fn equip(&mut self, item: UnitId, loc: u8) -> bool;
    /// Itemtype 5 (`bowq`).
    fn quiver(&mut self, item: UnitId) -> bool;
    /// Stat 70 := `n`.
    fn set_quantity(&mut self, item: UnitId, n: i32);
    /// Stat 72 := the max durability.
    fn fill_durability(&mut self, item: UnitId);
}

/// Creates the start items over the class's slots:
/// for each slot with a count, `count` copies, each created, finished
/// (steps 2.2–2.5) and placed (step 2.6). `start_skill`: charstats
/// `StartSkill` when it is a valid skill row (step 2.3, slot 0 only).
/// Returns each created item with where it went, in creation order.
pub fn start_items(
    w: &mut dyn StartItemWorld,
    slots: &[StartSlot],
    start_skill: Option<u16>,
) -> Vec<(UnitId, StartPlace)> {
    let mut out = Vec::new();
    for (s, slot) in slots.iter().enumerate() {
        if slot.count == 0 {
            continue;
        }
        for _ in 0..slot.count {
            // Step 1: a code not found makes every copy fail the same
            // lookup (no draw), so the slot ends here.
            let Some(item) = w.create(slot.code) else {
                break;
            };
            w.drop_class_skill_list(item);
            if s == 0 {
                if let Some(k) = start_skill {
                    w.set_single_skill(item, k);
                }
            }
            if w.stackable(item) {
                w.fill_stack(item);
            }
            w.mark_start(item, slot.loc);
            out.push((item, place_start_item(w, item, slot.loc)));
        }
    }
    out
}

/// Places one start item (step 2.6). A beltable item goes to the belt, else (failed) to
/// the inventory branch; loc 0: the inventory, then stat 70 := 250 for a
/// quiver (overridden by 100) else stat 72 := max durability; else equip
/// at loc, else (failed) the inventory placement, then stat 70 := 250 for
/// a quiver else stat 72 := max durability.
fn place_start_item(w: &mut dyn StartItemWorld, item: UnitId, loc: u8) -> StartPlace {
    let belted = w.beltable(item);
    if belted && w.place_belt(item) {
        return StartPlace::Belt;
    }
    if belted || loc == 0 {
        let stored = w.place_inventory(item);
        if w.quiver(item) {
            w.set_quantity(item, QUIVER_EQUIPPED);
            w.set_quantity(item, QUIVER_STORED);
        } else {
            w.fill_durability(item);
        }
        return if stored {
            StartPlace::Inventory
        } else {
            StartPlace::Nowhere
        };
    }
    let place = if w.equip(item, loc) {
        StartPlace::Equipped(loc)
    } else if w.place_inventory(item) {
        StartPlace::Inventory
    } else {
        StartPlace::Nowhere
    };
    if w.quiver(item) {
        w.set_quantity(item, QUIVER_EQUIPPED);
    } else {
        w.fill_durability(item);
    }
    place
}

/// Load result → (m, string id) of the refusal message the client shows.
/// `alt_bit` is the host config's alternate-message bit: results 17 and 18 then show
/// 0x5522 / 0x5521. A result outside 1..=26 gives m = 9 and string 5372.
pub fn refusal_message(result: u32, alt_bit: bool) -> (u8, u16) {
    // (m, string id) for results 1..=26, in order.
    const T: [(u8, u16); 26] = [
        (0, 5365),
        (1, 5366),
        (2, 5367),
        (3, 5368),
        (4, 5369),
        (5, 5371),
        (10, 5373),
        (11, 5374),
        (12, 5375),
        (13, 5376),
        (14, 5377),
        (15, 5378),
        (16, 5379),
        (17, 5380),
        (18, 5381),
        (19, 5360),
        (20, 5364),
        (21, 5363),
        (22, 5362),
        (23, 5361),
        (24, 5359),
        (9, 5372),
        (25, 10101),
        (26, 10102),
        (27, 5370),
        (28, 5371),
    ];
    match result {
        1..=26 => {
            let (m, id) = T[result as usize - 1];
            match (alt_bit, m) {
                (true, 20) => (m, 0x5522),
                (true, 21) => (m, 0x5521),
                _ => (m, id),
            }
        }
        _ => (9, 5372),
    }
}
pub mod save;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_fitems;

#[cfg(test)]
mod start_items_tests {
    use super::*;

    /// Codes: `hp1 ` beltable, `bowq` quiver, `jav ` stackable weapon,
    /// `bad ` not found; `full` refuses every placement.
    #[derive(Default)]
    struct Fake {
        next: u32,
        codes: Vec<(UnitId, [u8; 4])>,
        log: Vec<String>,
        belt_full: bool,
        equip_fails: bool,
    }

    impl Fake {
        fn code(&self, item: UnitId) -> [u8; 4] {
            self.codes.iter().find(|c| c.0 == item).unwrap().1
        }
        fn say(&mut self, item: UnitId, what: &str) {
            let c = String::from_utf8_lossy(&self.code(item)).trim().to_string();
            self.log.push(format!("{c} {what}"));
        }
    }

    impl StartItemWorld for Fake {
        fn create(&mut self, code: [u8; 4]) -> Option<UnitId> {
            if &code == b"bad " {
                return None;
            }
            self.next += 1;
            let u = UnitId(self.next);
            self.codes.push((u, code));
            self.say(u, "create");
            Some(u)
        }
        fn drop_class_skill_list(&mut self, item: UnitId) {
            self.say(item, "drop40");
        }
        fn set_single_skill(&mut self, item: UnitId, skill: u16) {
            self.say(item, &format!("skill{skill}"));
        }
        fn stackable(&mut self, item: UnitId) -> bool {
            matches!(&self.code(item), b"jav " | b"bowq")
        }
        fn fill_stack(&mut self, item: UnitId) {
            self.say(item, "stack");
        }
        fn mark_start(&mut self, item: UnitId, loc: u8) {
            self.say(item, &format!("start{loc}"));
        }
        fn beltable(&mut self, item: UnitId) -> bool {
            &self.code(item) == b"hp1 "
        }
        fn place_belt(&mut self, item: UnitId) -> bool {
            self.say(item, "belt");
            !self.belt_full
        }
        fn place_inventory(&mut self, item: UnitId) -> bool {
            self.say(item, "inv");
            &self.code(item) != b"full"
        }
        fn equip(&mut self, item: UnitId, loc: u8) -> bool {
            self.say(item, &format!("equip{loc}"));
            !self.equip_fails
        }
        fn quiver(&mut self, item: UnitId) -> bool {
            &self.code(item) == b"bowq"
        }
        fn set_quantity(&mut self, item: UnitId, n: i32) {
            self.say(item, &format!("qty{n}"));
        }
        fn fill_durability(&mut self, item: UnitId) {
            self.say(item, "dur");
        }
    }

    fn slot(code: &[u8; 4], loc: u8, count: u8) -> StartSlot {
        StartSlot {
            code: *code,
            loc,
            count,
        }
    }

    // Checks the creation order of the steps per slot, the slot counts and that
    // only slot 0 gets the start skill.
    #[test]
    fn slots_run_in_order_with_their_counts_and_the_start_skill_on_slot_0() {
        let mut w = Fake::default();
        let slots = [
            slot(b"jav ", 4, 1),
            slot(b"bad ", 0, 3),
            slot(b"hp1 ", 0, 2),
            slot(b"cap ", 1, 0),
            slot(b"bowq", 5, 1),
        ];
        let got = start_items(&mut w, &slots, Some(36));
        assert_eq!(
            got.iter().map(|g| g.1).collect::<Vec<_>>(),
            [
                StartPlace::Equipped(4),
                StartPlace::Belt,
                StartPlace::Belt,
                StartPlace::Equipped(5),
            ]
        );
        assert_eq!(
            w.log,
            [
                "jav create",
                "jav drop40",
                "jav skill36",
                "jav stack",
                "jav start4",
                "jav equip4",
                "jav dur",
                "hp1 create",
                "hp1 drop40",
                "hp1 start0",
                "hp1 belt",
                "hp1 create",
                "hp1 drop40",
                "hp1 start0",
                "hp1 belt",
                "bowq create",
                "bowq drop40",
                "bowq stack",
                "bowq start5",
                "bowq equip5",
                "bowq qty250",
            ]
        );
    }

    // Checks the fallbacks when the belt or the equip refuses an item.
    #[test]
    fn failed_belt_and_equip_fall_back_to_the_inventory() {
        let mut w = Fake {
            belt_full: true,
            equip_fails: true,
            ..Fake::default()
        };
        let slots = [
            slot(b"hp1 ", 0, 1),
            slot(b"bowq", 5, 1),
            slot(b"bowq", 0, 1),
            slot(b"full", 3, 1),
        ];
        let got = start_items(&mut w, &slots, None);
        assert_eq!(
            got.iter().map(|g| g.1).collect::<Vec<_>>(),
            [
                StartPlace::Inventory,
                StartPlace::Inventory,
                StartPlace::Inventory,
                StartPlace::Nowhere,
            ]
        );
        let tail = |c: &str| {
            w.log
                .iter()
                .filter(|l| l.starts_with(c) && !l.contains("create"))
                .cloned()
                .collect::<Vec<_>>()
        };
        // Belt refused → the inventory branch (no start skill: none given).
        assert_eq!(
            tail("hp1"),
            ["hp1 drop40", "hp1 start0", "hp1 belt", "hp1 inv", "hp1 dur"]
        );
        // An equipped quiver gets 250 even when the equip falls back; a
        // stored one gets 250 then 100.
        assert_eq!(
            tail("bowq"),
            [
                "bowq drop40",
                "bowq stack",
                "bowq start5",
                "bowq equip5",
                "bowq inv",
                "bowq qty250",
                "bowq drop40",
                "bowq stack",
                "bowq start0",
                "bowq inv",
                "bowq qty250",
                "bowq qty100",
            ]
        );
        assert_eq!(
            tail("full"),
            [
                "full drop40",
                "full start3",
                "full equip3",
                "full inv",
                "full dur"
            ]
        );
    }
}
