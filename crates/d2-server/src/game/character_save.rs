//! Saving a played character: what the running game says about a player
//! ([`read_live`]), laid over the save it was loaded from ([`apply_live`]).
//! Single player writes the result to the player's `.d2s`; the dedicated
//! server writes it to the character's file on the server.
//!
//! What a save holds today (D2RUST's own, unverified): the character the
//! join loaded (a parsed save, or a fresh header for a new character) with
//! the live values the load restores overlaid: the 16 base stats
//! ([`SNAPSHOT_STATS`]; `ActionCharacter::set_base_stat` loads them back),
//! the quest flag records, items, skill levels, waypoints and the
//! [`save_gaps`] records. What the running game cannot hold or read back
//! passes through from the loaded save unchanged, and is empty for a new
//! character.

use d2_formats::d2s::appearance::{AppearanceTables, IsA, ReferenceSlots};
use d2_formats::d2s::{self, Body, D2s, Header, StatEntry, Stats};
use d2_sim::units::UnitId;

use super::{save_full, save_gaps, Character, Sim, GAME_SETUP, PLAYER_CLASS, PLAYER_NAME};
use crate::adapters::character::save::SaveContext;

/// The base stats saved from the live player (strength.. goldbank, stat ids
/// 0..=15).
pub const SNAPSHOT_STATS: [u16; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// Stat id of the character level.
pub const LEVEL_STAT: u16 = 12;

/// The save the live values are laid over: a loaded save as it is, or
/// the stub of a new character (the writer's defaults: its own header,
/// one empty body) with its class and name.
// D2RUST's own, unverified
pub fn base_save(character: &Character) -> D2s {
    match character {
        Character::Save(save, _) => (**save).clone(),
        Character::Named(c) => fresh(c.class, c.name(), c.difficulty),
        Character::Refused(r) => fresh(
            0,
            &r.name[..r.name.iter().position(|&b| b == 0).unwrap_or(16)],
            r.difficulty,
        ),
        Character::New => fresh(PLAYER_CLASS as u8, PLAYER_NAME, GAME_SETUP.difficulty),
    }
}

/// The stub save of a new character of `class` named `name` on
/// `difficulty`: the writer's defaults (its own header, one empty body).
pub fn fresh(class: u8, name: &[u8], difficulty: u8) -> D2s {
    let mut header = Header {
        class,
        ..Header::default()
    };
    let _ = header.set_name(name);
    // Not `status::NEW`: that flag marks the 335-byte stub, and
    // this file has a body.
    header.status = if GAME_SETUP.expansion {
        d2s::status::EXPANSION
    } else {
        0
    };
    // Town per difficulty: act 0 of the current one.
    header.towns[usize::from(difficulty).min(2)] = 0x80;
    // An expansion game's file always
    // ends `6A 66 6B 66 00` when there is no hireling and no golem.
    let expansion = header.status & d2s::status::EXPANSION != 0;
    D2s {
        header,
        body: Some(Body {
            skills: vec![0; usize::from(header_skill_count())],
            hireling_items: expansion.then_some(None),
            golem: expansion.then(d2s::Golem::default),
            ..Body::default()
        }),
    }
}

fn header_skill_count() -> u8 {
    Header::default().skill_count
}

/// Now, in Unix seconds (a save's save time).
pub fn now_secs() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32)
}

/// One NPC bit field per difficulty.
pub type NpcField = [[u8; 8]; 3];

/// What the running game says about the player at save time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Live {
    /// `(stat id, base value)` for [`SNAPSHOT_STATS`].
    pub stats: Vec<(u16, i32)>,
    /// The quest flag records per difficulty (`QuestFlags::copy_out`).
    pub quests: Option<[[u8; 96]; 3]>,
    /// Items, skill levels and waypoints (q-save-full).
    pub extra: super::save_full::Extra,
    /// Mouse skills, town act, hireling and golem items (q-save-gaps).
    pub gaps: super::save_gaps::Gaps,
    /// The character is hardcore and the player is dying or dead
    /// ([`super::hardcore`]): the save gets the dead bit.
    pub hardcore_dead: bool,
    /// The character is hardcore.
    pub hardcore: bool,
    /// The status bits the game set on the player's client
    /// (`ClientEntry::status_set`: 0x08 at every
    /// death start, softcore too); the save ORs them in.
    pub status_set: u16,
    /// The game's map seed (game +0x7C, the DRLG init seed of act 0:
    /// the save's +0xAB); `None`: no DRLG (synthetic data).
    pub map_seed: Option<u32>,
    /// The NPC fields A (first talk) and B (introduced) per difficulty
    /// (`PlayerQuests::first_talk` / `intro`); `None`: no quest record for the
    /// player.
    pub npcs: Option<(NpcField, NpcField)>,
}

/// Reads [`Live`] from the running game's `player`.
pub fn read_live(sim: &mut Sim, player: UnitId) -> Live {
    let mut stats: Vec<(u16, i32)> = sim.events.action.with(&mut sim.game, |_, v| {
        SNAPSHOT_STATS
            .iter()
            .map(|&id| (id, v.stats.unit_base(player, id, 0)))
            .collect()
    });
    // A player always has level >= 1. Level 0 means the game holds no
    // stat table (the synthetic game): its zeros must not overwrite the
    // loaded character's stats.
    if stats.iter().any(|&(id, v)| id == LEVEL_STAT && v <= 0) {
        stats.clear();
    }
    let npcs = sim
        .world
        .rest
        .quests
        .get(&player)
        .map(|q| (q.first_talk, [0, 1, 2].map(|d| q.intro_bits(d))));
    let quests = sim.world.rest.quests.get(&player).map(|q| {
        let mut out = [[0u8; 96]; 3];
        for (rec, f) in out.iter_mut().zip(&q.flags) {
            *rec = f.copy_out();
        }
        out
    });
    let extra = save_full::read_extra(sim, player);
    let gaps = save_gaps::read_gaps(sim, player);
    let hardcore = sim.events.action.hooks().x.hardcore;
    // Player modes 0 (DT) and 17 (DD).
    let down = sim
        .events
        .action
        .sys
        .units
        .get(player)
        .is_some_and(|u| u.mode == 0 || u.mode == 17);
    let status_set = sim
        .game
        .lists
        .clients()
        .into_iter()
        .filter_map(|c| sim.game.lists.client(c))
        .filter(|e| e.player == Some(player))
        .fold(0, |a, e| a | e.status_set);
    let map_seed = sim
        .events
        .action
        .sys
        .hooks
        .drlg
        .dungeon
        .acts
        .first()
        .and_then(Option::as_ref)
        .map(|d| d.init_seed);
    Live {
        npcs,
        map_seed,
        stats,
        quests,
        extra,
        gaps,
        hardcore,
        hardcore_dead: hardcore && down,
        status_set,
    }
}

/// `base` with the live values laid over it (the module docs list what).
// D2RUST's own, unverified
pub fn apply_live(base: &D2s, live: &Live, now: u32) -> D2s {
    let mut save = base.clone();
    let body = save.body.get_or_insert_with(Body::default);
    match &mut body.stats {
        Stats::Bits(entries) => {
            for &(id, value) in &live.stats {
                match entries.iter_mut().find(|e| e.id == id && e.layer == 0) {
                    Some(e) => e.value = value,
                    None if value != 0 => entries.push(StatEntry {
                        id,
                        layer: 0,
                        value,
                    }),
                    None => {}
                }
            }
            entries.retain(|e| e.value != 0);
            entries.sort_by_key(|e| (e.id, e.layer));
        }
        Stats::Mask { values, .. } => {
            for &(id, value) in &live.stats {
                if let Some(v) = values.iter_mut().find(|v| v.0 == id) {
                    v.1 = value as u32;
                }
            }
        }
    }
    if let Some(records) = &live.quests {
        body.quests.records = *records;
    }
    // The writer copies A and B of each difficulty.
    if let Some((a, b)) = live.npcs {
        body.npcs.a = a;
        body.npcs.b = b;
    }
    save_full::apply_extra(body, &live.extra);
    save_gaps::apply_gaps(&mut save, &live.gaps);
    if let Some(&(_, level)) = live.stats.iter().find(|s| s.0 == LEVEL_STAT) {
        if level > 0 {
            save.header.level = level.min(99) as u8;
        }
    }
    if live.hardcore {
        save.header.status |= d2s::status::HARDCORE;
    }
    // The client word's bits set in game (the writer copies the client word,
    // which no code clears).
    save.header.status |= live.status_set;
    // A character started from the new-character stub is saved with a
    // body from its first save on, so the stub's "new" flag goes: the
    // loader only accepts that flag on a bare header.
    save.header.status &= !d2s::status::NEW;
    // +0xAB is game +0x7C: the seed the game was built with.
    if let Some(seed) = live.map_seed {
        save.header.map_seed = seed;
    }
    mark_dead(&mut save, live.hardcore, live.hardcore_dead);
    // +0x2C stays as loaded: the game never sets the create time, so every
    // game-written save holds 0 there.
    // The game writes only 0x60, with the bit-field
    // stats; a loaded 0x5C–0x5E file is upgraded (its item records are
    // in the sim's format, or the store refuses the save,
    // [`old_items_pass_through`]).
    if save.header.version < d2s::VERSION {
        save.header.version = d2s::VERSION;
        if let Some(body) = save.body.as_mut() {
            if let Stats::Mask { .. } = body.stats {
                let mut entries = body.stats.entries();
                entries.sort_by_key(|e| (e.id, e.layer));
                body.stats = Stats::Bits(entries);
            }
        }
    }
    save.header.save_time = now;
    save
}

/// `save`'s header marked as a dead hardcore character: a hardcore header
/// that has died gets the dead bit; anything else is left alone.
pub fn mark_dead(save: &mut D2s, hardcore: bool, died: bool) {
    if hardcore && died {
        save.header.status |= d2s::status::HARDCORE | d2s::status::DEAD;
    }
}

/// The appearance tables of `fixed` with the image's reference table
/// (`ReferenceSlots::game`).
pub fn appearance_tables(fixed: &d2_data::fixup::FixedSet) -> Result<AppearanceTables, String> {
    let eq = &fixed.itemtypes_equiv;
    let m = IsA::from_fn(eq.n, |i, j| eq.get(i, j));
    crate::world_data::tables::appearance_tables(fixed, &ReferenceSlots::game(&m))
        .map_err(|e| e.to_string())
}

/// The save's appearance rules (the writer, per
/// equipped item): the appearance bytes of `player` from its items, its
/// weapon class and the colour states it has.
pub fn rebuild_appearance(sim: &mut Sim, player: UnitId, t: &AppearanceTables, save: &mut D2s) {
    use d2_sim::wiring::action::Pending;
    let weapon_class = sim.events.action.hooks().x.composit_weapon_class(player);
    let states = t
        .colours
        .states
        .iter()
        .filter(|s| sim.events.action.sys.stats.has_state(player, s.state))
        .map(|s| s.state)
        .collect();
    let ctx = SaveContext {
        expansion: GAME_SETUP.expansion,
        weapon_class,
        states,
    };
    if let Ok(eq) = sim
        .world
        .save_equipment(&mut sim.game, &mut sim.events, player, &ctx)
    {
        save.header.rebuild_appearance(&eq, t);
    }
}

/// Whether a save of `base` (a loaded file) with `live` would carry item
/// records as loaded: a list the running game cannot read back (no inventory
/// model, no hireling or golem unit) passes through. For a 0x5C–0x5E file those
/// records are in the old bit layout and cannot sit under the 0x60 header the
/// writer gives the file.
pub fn old_items_pass_through(base: &D2s, live: &Live) -> bool {
    let Some(b) = &base.body else {
        return false;
    };
    (live.extra.items.is_none() && !b.items.is_empty())
        || (live.extra.corpses.is_none() && b.corpses.iter().any(|c| !c.items.is_empty()))
        || (live.gaps.hireling_items.is_none()
            && matches!(&b.hireling_items, Some(Some(v)) if !v.is_empty()))
        || (live.gaps.golem.is_none() && b.golem.as_ref().is_some_and(|g| g.item.is_some()))
}
