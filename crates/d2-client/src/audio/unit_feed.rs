//! The unit sounds of the client model: the table rows the rules read
//! ([`UnitSoundRows`]: `monsounds`, `monstats`, `monstats2`,
//! `superuniques`, `skills`, `states`, `missiles`, the item tables,
//! `AnimData`) and the per-unit pass ([`UnitFeed`]) that turns the model
//! into the unit sound calls:
//!
//! - a **mode change** of a player or monster runs the mode sounds
//!   (hit / death, swings, blocks, kicks, monster attack / skill
//!   voices, the dead-mode voice stop),
//! - a **new monster** runs the `Init` voice,
//! - **every client update** runs the monster umod hooks,
//!   the monster `Neutral` voice and
//!   the footsteps,
//! - a state bit turning on or off runs the state sounds,
//! - a **new missile** requests its `TravelSound`,
//! - an item **dropping** (mode 5) or reaching the cursor (mode 4) runs
//!   the item sounds,
//! - S→C 0x2C events 12, 16, 17 read the record the pass chooses
//!   ([`UnitFeed::event_unit`], [`UnitFeed::event12_stsound`]).
//!
//! The animation clock the footsteps read: f := 0 at a mode change,
//! wrapping at F (frames × 256); the speed is the unit's animation rate
//! (a monster from the client model,
//! `bridge::monster_anim`, which also rolls a new monster's first frame;
//! a player through [`player_rate`]); a monster's footstep and a
//! player's both read f before the update's advance. A monster
//! the model does not animate (no 0xAC set-up: synthetic fixtures) keeps
//! the raw AnimData speed from f = 0.
//!
//! D2RUST's own choices where the rule is not pinned down:
//!
//! - **Order of units in one update:** ascending unit key (the game
//!   walks the room unit lists).
//! - **Weapon hit class of a player:** the right-hand body item (body
//!   location 4) by its code in `weapons` `hit class`; the weapon swap
//!   slots and a 2nd-hand weapon are not read.
//! - **The first sight of a unit:** a monster seen for the first time
//!   plays `Init` and the sounds of the states it already has; an item
//!   first seen dropping (mode 5) plays the drop sound. The game runs
//!   them at a creation call the model does not distinguish from a level
//!   load.
//! - **Missile `HitSound`, `ProgSound`:** they need the result of the
//!   client hit / progressive function, which the model does not run;
//!   not requested.
//! - **Skill start sounds:** a unit's mode request with code 0x15 or 0x16
//!   (the client skill start, record entry 0 = the skill) plays the
//!   skill's start sounds after the mode sounds, the start function taken
//!   as having returned non-zero (the model does not run `cltstfunc`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use d2_data::bin::TableFiles;
use d2_data::tables::{
    decode_all, Armor, Misc, Missiles, Monsounds, Monstats, Monstats2, Skills, States,
    Superuniques, Weapons,
};
use d2_formats::animdata::AnimData;

use crate::audio::triggers::hooks::umod_phase2;
use crate::audio::triggers::identity::{monsounds_row, RecordInputs};
use crate::audio::triggers::modes::{mode_set, mode_sound};
use crate::audio::triggers::movement::{
    footstep, footstep_called, footstep_material, init_voice, neutral, Floor, TOWN_LEVELS,
};
use crate::audio::triggers::skills::{skill_start, state_off, state_on, SkillStart};
use crate::audio::triggers::{within_700, Ctx, TriggerError, Unit, UnitSound, MONSTER, PLAYER};
use crate::bridge::disguise::FLAG_EX_DISGUISE;
use crate::bridge::items::{self, mode as item_mode};
use crate::bridge::world::{view_key, ClientUnit, ClientWorld, KindData, UnitKey, ITEM, MISSILE};
use crate::world_view::unit_assets::{unit_cof, UnitLooks};

/// The `monstats` columns the sound identity reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MonsterSound {
    base_id: i32,
    monsound: i32,
    umonsound: i32,
    /// The `monstats2` row (`MonStatsEx`).
    ex: usize,
}

/// The sound columns of a `skills` row used by event 12.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StateSound {
    on: i32,
    off: i32,
    notondead: bool,
}

/// The sound columns of an item row (`dropsound`, `dropsfxframe`,
/// `usesound`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemSoundRow {
    pub dropsound: i32,
    pub dropsfxframe: u32,
    pub usesound: i32,
}

/// A link16 as the signed id the rules compare (an unset link is all
/// ones, so −1).
fn s16(v: u16) -> i32 {
    i32::from(v as i16)
}

/// Clears every sound column of a `monsounds` row that names a sound without audio
/// ([`d2_data::sounds::SOUNDS_WITHOUT_AUDIO`]): the monster then has no such voice.
pub(crate) fn clear_sounds_without_audio(r: &mut Monsounds) {
    for id in [
        &mut r.attack1,
        &mut r.attack2,
        &mut r.weapon1,
        &mut r.weapon2,
        &mut r.hitsound,
        &mut r.deathsound,
        &mut r.skill1,
        &mut r.skill2,
        &mut r.skill3,
        &mut r.skill4,
        &mut r.footstep,
        &mut r.footsteplayer,
        &mut r.neutral,
        &mut r.init,
        &mut r.taunt,
        &mut r.flee,
    ] {
        if d2_data::sounds::has_no_audio(*id) {
            *id = 0;
        }
    }
}

/// The tables the unit sounds read, from the user's own files.
#[derive(Clone, Default)]
pub struct UnitSoundRows {
    monsounds: Vec<Monsounds>,
    monstats: Vec<MonsterSound>,
    critter: Vec<bool>,
    superunique_monsound: Vec<i32>,
    skill_stsound: Vec<i32>,
    /// The start-sound columns of each `skills` row.
    skill_start: Vec<SkillStart>,
    states: Vec<StateSound>,
    /// `TravelSound` of each missile row.
    missile_travel: Vec<i32>,
    /// `hit class` of each weapon, by code.
    weapon_hit_class: BTreeMap<[u8; 4], u8>,
    item_sounds: BTreeMap<[u8; 4], ItemSoundRow>,
    looks: Option<Arc<UnitLooks>>,
    anim: Option<Arc<AnimData>>,
}

impl std::fmt::Debug for UnitSoundRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UnitSoundRows({} monsounds, {} monstats)",
            self.monsounds.len(),
            self.monstats.len()
        )
    }
}

impl UnitSoundRows {
    /// The rows of the install behind `archives`; `looks` and `anim` give
    /// the animation (frames, speed) of a unit's mode.
    pub fn live(
        archives: &dyn TableFiles,
        looks: Arc<UnitLooks>,
        anim: Arc<AnimData>,
    ) -> Result<Self, String> {
        let set = d2_data::bin::load_from(archives, d2_data::bin::DEFAULT_LANGUAGE)
            .map_err(|e| e.to_string())?;
        let table = |name: &str| set.table(name).ok_or(format!("{name} not loaded"));
        let mut monsounds: Vec<Monsounds> =
            decode_all(table("monsounds")?).map_err(|e| e.to_string())?;
        monsounds.iter_mut().for_each(clear_sounds_without_audio);
        let monstats: Vec<Monstats> = decode_all(table("monstats")?).map_err(|e| e.to_string())?;
        let monstats2: Vec<Monstats2> =
            decode_all(table("monstats2")?).map_err(|e| e.to_string())?;
        let supers: Vec<Superuniques> =
            decode_all(table("superuniques")?).map_err(|e| e.to_string())?;
        let skills: Vec<Skills> = decode_all(table("skills")?).map_err(|e| e.to_string())?;
        let states: Vec<States> = decode_all(table("states")?).map_err(|e| e.to_string())?;
        let missiles: Vec<Missiles> = decode_all(table("missiles")?).map_err(|e| e.to_string())?;
        let weapons: Vec<Weapons> = decode_all(table("weapons")?).map_err(|e| e.to_string())?;
        let armor: Vec<Armor> = decode_all(table("armor")?).map_err(|e| e.to_string())?;
        let misc: Vec<Misc> = decode_all(table("misc")?).map_err(|e| e.to_string())?;
        let mut item_sounds = BTreeMap::new();
        let mut put = |code: [u8; 4], d: u16, f: u8, u: u16| {
            item_sounds.insert(
                code,
                ItemSoundRow {
                    dropsound: s16(d),
                    dropsfxframe: u32::from(f),
                    usesound: s16(u),
                },
            );
        };
        for w in &weapons {
            put(w.code, w.dropsound, w.dropsfxframe, w.usesound);
        }
        for a in &armor {
            put(a.code, a.dropsound, a.dropsfxframe, a.usesound);
        }
        for m in &misc {
            put(m.code, m.dropsound, m.dropsfxframe, m.usesound);
        }
        Ok(Self {
            monsounds,
            monstats: monstats
                .iter()
                .map(|m| MonsterSound {
                    base_id: i32::from(m.baseid),
                    monsound: s16(m.monsound),
                    umonsound: s16(m.umonsound),
                    ex: usize::from(m.monstatsex),
                })
                .collect(),
            critter: monstats2.iter().map(|m| m.critter).collect(),
            superunique_monsound: supers.iter().map(|s| s.monsound as i32).collect(),
            skill_stsound: skills.iter().map(|s| s16(s.stsound)).collect(),
            skill_start: skills
                .iter()
                .map(|s| SkillStart {
                    stsound: s16(s.stsound),
                    stsoundclass: s16(s.stsoundclass),
                    stsounddelay: s.stsounddelay,
                    weaponsnd: s.weaponsnd,
                    stsuccessonly: s.stsuccessonly,
                    charclass: i32::from(s.charclass as i8),
                    item_cast_sound: None,
                })
                .collect(),
            states: states
                .iter()
                .map(|s| StateSound {
                    on: s16(s.onsound),
                    off: s16(s.offsound),
                    notondead: s.notondead,
                })
                .collect(),
            missile_travel: missiles.iter().map(|m| s16(m.travelsound)).collect(),
            weapon_hit_class: weapons.iter().map(|w| (w.code, w.hit_class)).collect(),
            item_sounds,
            looks: Some(looks),
            anim: Some(anim),
        })
    }

    /// The `monsounds` row of a monster. The sound identity of a monster
    /// is its own type and class.
    pub fn record(&self, class: u32, flags: u8, hc_idx: u16) -> Option<&Monsounds> {
        self.monsounds.get(self.record_index(class, flags, hc_idx)?)
    }

    /// The row index of [`Self::record`].
    pub fn record_index(&self, class: u32, flags: u8, hc_idx: u16) -> Option<usize> {
        let row = self.monstats.get(class as usize);
        let inputs = RecordInputs {
            raw_type: MONSTER,
            class: class as i32,
            type_flags: flags,
            superunique_monsound: self.superunique_monsound.get(usize::from(hc_idx)).copied(),
            umonsound: row.map_or(0, |r| r.umonsound),
            monsound: row.map_or(-1, |r| r.monsound),
            monstats_rows: self.monstats.len() as i32,
            monsounds_rows: self.monsounds.len() as i32,
        };
        usize::try_from(monsounds_row(&inputs)?).ok()
    }

    fn base_class(&self, class: u32) -> i32 {
        self.monstats
            .get(class as usize)
            .map_or(class as i32, |m| m.base_id)
    }

    fn critter(&self, class: u32) -> bool {
        self.monstats
            .get(class as usize)
            .and_then(|m| self.critter.get(m.ex))
            .copied()
            .unwrap_or(false)
    }

    /// (`onsound`, `offsound`) of a state.
    pub fn state_sounds(&self, state: u8) -> Option<(i32, i32)> {
        self.states.get(usize::from(state)).map(|s| (s.on, s.off))
    }

    /// `TravelSound` of a missile row.
    pub fn missile_travel(&self, class: u32) -> Option<i32> {
        self.missile_travel.get(class as usize).copied()
    }

    /// The sound columns of an item code.
    pub fn item_sound(&self, code: [u8; 4]) -> Option<ItemSoundRow> {
        self.item_sounds.get(&code).copied()
    }

    /// The start-sound columns of a skill.
    pub fn skill_start_row(&self, skill: u16) -> Option<SkillStart> {
        self.skill_start.get(usize::from(skill)).copied()
    }

    /// `stsound` of a skill (event 12).
    pub fn skill_stsound(&self, skill: u16) -> i32 {
        self.skill_stsound
            .get(usize::from(skill))
            .copied()
            .unwrap_or(0)
    }

    /// The (frame count F, speed) of the animation of `u` in its mode, as
    /// the 8.8 values the footstep rule reads.
    pub fn animation(&self, u: &ClientUnit) -> Option<(u32, i32)> {
        let (looks, anim) = (self.looks.as_ref()?, self.anim.as_ref()?);
        let name = unit_cof(looks, u)?.short().to_ascii_uppercase();
        let b = name.as_bytes();
        if b.len() > 8 {
            return None;
        }
        let mut key = [0u8; 8];
        key[..b.len()].copy_from_slice(b);
        let info = anim.info(&key).ok()?;
        info.found
            .then(|| (info.frames.wrapping_mul(256), i32::from(info.speed as i16)))
    }
}

/// A player's animation speed (through `d2_sim::units::anim_rate`): draw
/// type 0, the AnimData speed `s`, the unit's totals of stats 67–69,
/// w = 101 in mode 3 (run) else 213, the mode row's V column as the
/// velocity-mode test. The amazon's TW runs at 213 (AnimData 256).
fn player_rate(world: &ClientWorld, key: UnitKey, u: &ClientUnit, s: i32) -> i32 {
    use d2_sim::units::anim_rate::{anim_rate, mode_row, Rate, RateInput};
    let m = u.mode;
    // A model player without a base stat reads the creation base 100
    // under its lists, as the walk prediction (`bridge::predict`).
    let total = |stat: u16| {
        let base = if u.stats.contains_key(&stat) { 0 } else { 100 };
        base + world.total(key, stat, 0)
    };
    let i = RateInput {
        applies: true,
        t: 0,
        c: u.class,
        m,
        s,
        velocitypercent: total(67),
        attackrate: total(68),
        other_animrate: total(69),
        has_path: true,
        w: if m == 3 { 101 } else { 213 },
        velocity_mode: mode_row(0, u.class, m).v,
        ..RateInput::default()
    };
    match anim_rate(&i) {
        Ok(Rate::Set { speed, .. }) => speed,
        _ => s,
    }
}

/// What the pass remembers of a unit between frames.
#[derive(Clone, Debug, Default)]
struct Track {
    mode: u32,
    /// +0x44 / +0x48 / +0x4C.
    frame: u32,
    frame_count: u32,
    speed: i32,
    states: BTreeSet<u8>,
    /// `ClientUnit::mode_requests` at the last pass (skill starts and the
    /// explicit 0x13 sound read it).
    mode_requests: u32,
    /// `ClientUnit::anim_restarts` at the last pass.
    anim_restarts: u32,
    /// `ClientUnit::mode_changes` at the last pass.
    mode_changes: u32,
    /// `ClientUnit::mode_ends` at the last pass.
    mode_ends: u32,
    /// The local player's click start, held one sound tick: the click runs
    /// in the loop pass after that pass's sound tick, so its request is
    /// served by the next tick (a Frost Nova requested at f 19 plays at
    /// f 20).
    held_start: Option<u16>,
}

/// One unit of a frame with the inputs the rules read.
struct Planned {
    key: UnitKey,
    /// A set C unit (a client-only critter).
    client_only: bool,
    first: bool,
    mode_changed: bool,
    /// A monster whose mode set restarted its animation since the last
    /// pass with the mode the same as before (2 → 1 → 2 within a frame).
    restarted: bool,
    /// A monster whose mode changed and changed back since the last pass
    /// (14 → 1 → 14: a repeated attack): each real change runs the mode
    /// sounds.
    reentered: bool,
    /// The explicit m of a new code 0x13 mode request (a hit with no mode
    /// change).
    explicit: Option<u8>,
    /// +0xB0, the hit class of the unit's last hit request.
    hit_class: u8,
    states_on: Vec<u8>,
    states_off: Vec<u8>,
    is_local: bool,
    base_class: i32,
    critter: bool,
    dying: bool,
    state_146: bool,
    frozen: bool,
    weapon_hit_class: u8,
    near_local: bool,
    local_dist: i32,
    record: Option<usize>,
    item: Option<(Option<ItemSoundRow>, u8)>,
    /// The skill of a client skill start request (code 0x15 / 0x16, record
    /// entry 0) the unit received since the last pass.
    skill_request: Option<u16>,
    /// A monster with a mode request since the last pass (other than a
    /// change to NU): the request sits in the unit's queue, drained
    /// after the unit's update,
    /// so that update's idle voice
    /// reads `prev_mode` and the request's sounds follow it
    /// (the mode read of the event rules).
    queued: bool,
    /// The mode the last pass saw.
    prev_mode: u8,
    /// The walk that ended at this pass, the local player's or a
    /// monster's stopped by the server: the walk mode
    /// and its (f, F, s) before the switch to neutral.
    walk_end: Option<(u8, u32, u32, i32)>,
}

/// The per-unit sound pass state.
#[derive(Default)]
pub struct UnitFeed {
    rows: Option<Arc<UnitSoundRows>>,
    /// Keyed by the unit and whether it is in set C (a client-only
    /// critter): a set C GUID may equal a set S
    /// one.
    tracks: BTreeMap<(UnitKey, bool), Track>,
    /// The local click already reaches the model one pass late (a check's
    /// input script following a given frame schedule on a drawn tick):
    /// the one-tick hold of
    /// [`Track::held_start`] is that same delay, so it is not applied twice.
    click_late: bool,
    /// State bits whose on-sound [`UnitFeed::state_prepass`] already made
    /// this frame; the unit pass of the same frame skips them.
    pre_done: BTreeSet<(UnitKey, u8)>,
}

impl UnitFeed {
    /// The on-sounds of the states of units new to the sound layer, in the
    /// order the states were turned on (`ClientWorld::state_on_seq`): the
    /// game requests them from the packet handler, so a creation frame's
    /// state sounds come before the game-start sound and the other
    /// requests of the frame (player, then the monsters in packet order,
    /// each id 0 for state 105).
    pub fn state_prepass(&mut self, cx: &mut Ctx, world: &ClientWorld) {
        let Some(rows) = self.rows.clone() else {
            return;
        };
        let mut todo: Vec<(u64, UnitKey, u8)> = Vec::new();
        for (&key, u) in &world.units {
            if !matches!(key.unit_type, PLAYER | MONSTER) || self.tracks.contains_key(&(key, false))
            {
                continue;
            }
            for &s in &u.states {
                let seq = world.state_on_seq.get(&(key, s)).copied().unwrap_or(0);
                todo.push((seq, key, s));
            }
        }
        todo.sort();
        for (_, key, s) in todo {
            let st = rows.states.get(usize::from(s)).copied().unwrap_or_default();
            let dead = world.units.get(&key).is_some_and(ClientUnit::is_dead);
            state_on(cx, key, false, st.notondead, dead, st.on);
            self.pre_done.insert((key, s));
        }
    }

    /// Whether this frame's local click was already delayed one pass by the
    /// input layer (see [`UnitFeed::click_late`]).
    pub fn set_click_late(&mut self, late: bool) {
        self.click_late = late;
    }
    pub fn set_rows(&mut self, rows: Arc<UnitSoundRows>) {
        self.rows = Some(rows);
    }

    pub fn rows(&self) -> Option<&Arc<UnitSoundRows>> {
        self.rows.as_ref()
    }

    /// The monster's `monsounds` record for the event rules (events 16,
    /// 17).
    pub fn event_record(&self, world: &ClientWorld, key: UnitKey) -> Option<Monsounds> {
        let rows = self.rows.as_ref()?;
        let u = world.units.get(&key)?;
        if key.unit_type != MONSTER {
            return None;
        }
        let KindData::Monster(m) = &u.kind else {
            return rows.record(u.class, 0, 0).cloned();
        };
        rows.record(u.class, m.flags, m.hc_idx).cloned()
    }

    /// Event 12: the `stsound` of the skill in stat
    /// 350 of U's state-68 stat list, when U has that skill; 0 otherwise.
    pub fn event12_stsound(&self, world: &ClientWorld, key: UnitKey) -> i32 {
        let Some(rows) = self.rows.as_ref() else {
            return 0;
        };
        let Some(u) = world.units.get(&key) else {
            return 0;
        };
        let Some(list) = u.state_lists.get(&68) else {
            return 0;
        };
        let skill = list
            .iter()
            .find(|((stat, _), _)| *stat == 350)
            .map(|(_, &v)| v);
        let Some(skill) = skill.and_then(|v| u16::try_from(v).ok()) else {
            return 0;
        };
        let has = u
            .skills
            .as_ref()
            .is_some_and(|l| l.entries.iter().any(|e| e.skill == skill));
        if has {
            rows.skill_stsound(skill)
        } else {
            0
        }
    }

    /// The unit as the rules of an event read it.
    pub fn event_unit<'a>(
        &self,
        world: &ClientWorld,
        key: UnitKey,
        class: u32,
        record: Option<&'a Monsounds>,
    ) -> Unit<'a> {
        let mut u = Unit::new(key, class as i32);
        u.is_local = world.local_player == Some(key);
        if let Some(c) = world.units.get(&key) {
            if u.is_local {
                u.mode = c.mode as u8;
            }
        }
        u.monsounds = record;
        u
    }

    /// Plans a frame: every sound-relevant unit with the changes since the
    /// last frame. `position` is the sound world's client pixel point.
    fn plan(
        &mut self,
        world: &ClientWorld,
        position: &dyn Fn(UnitKey) -> Option<(i32, i32)>,
        drawn: Option<(UnitKey, u32)>,
    ) -> Vec<Planned> {
        let Some(rows) = self.rows.clone() else {
            return Vec::new();
        };
        let pre = std::mem::take(&mut self.pre_done);
        let local = world.local_player;
        let local_at = local.and_then(position);
        let item_views: BTreeMap<UnitKey, items::ItemView> = items::items(world)
            .into_iter()
            .map(|v| (v.key, v))
            .collect();
        // The right-hand body item of the local player.
        let local_weapon = local.and_then(|p| {
            item_views
                .values()
                .find(|v| v.owner == Some(p) && v.mode == item_mode::BODY && v.body == 4)
                .and_then(|v| v.code)
                .and_then(|c| rows.weapon_hit_class.get(&c).copied())
        });
        let mut out = Vec::new();
        let mut live = BTreeSet::new();
        for (key, u, client_only) in sound_units(world) {
            if !matches!(key.unit_type, PLAYER | MONSTER | MISSILE | ITEM) {
                continue;
            }
            live.insert((key, client_only));
            let first = !self.tracks.contains_key(&(key, client_only));
            let t = self.tracks.entry((key, client_only)).or_default();
            let mode = effective_mode(u, drawn);
            let mode_changed = !first && t.mode != mode;
            let restarted = !first
                && !mode_changed
                && key.unit_type == MONSTER
                && t.anim_restarts != u.anim_restarts;
            let reentered = restarted && t.mode_changes != u.mode_changes;
            let explicit = explicit_mode_sound(u, first, t.mode_requests);
            let states_on: Vec<u8> = u
                .states
                .iter()
                .copied()
                .filter(|s| !t.states.contains(s) && !pre.contains(&(key, *s)))
                .collect();
            let states_off: Vec<u8> = t
                .states
                .iter()
                .copied()
                .filter(|s| !u.states.contains(s))
                .collect();
            let is_local = local == Some(key);
            let (near_local, local_dist) = match (position(key), local_at) {
                (Some((x, y)), Some((px, py))) => (
                    within_700(i64::from(x - px), i64::from(y - py)),
                    ((x - px).abs()).max((y - py).abs()),
                ),
                _ => (false, 0),
            };
            let mut p = Planned {
                key,
                client_only,
                first,
                mode_changed,
                restarted,
                reentered,
                explicit,
                hit_class: u.hit_class as u8,
                states_on,
                states_off,
                is_local,
                base_class: u.class as i32,
                critter: false,
                dying: u.is_dead(),
                state_146: u.states.contains(&146),
                frozen: u.states.contains(&1),
                weapon_hit_class: 0,
                near_local,
                local_dist,
                record: None,
                item: None,
                skill_request: None,
                // A change to NU is the client's own mode end inside the
                // update, whatever the server
                // sent with it.
                queued: key.unit_type == MONSTER
                    && !first
                    && u.mode_requests != t.mode_requests
                    && !(mode_changed && mode == 1),
                prev_mode: t.mode as u8,
                walk_end: None,
            };
            if (is_local && mode_changed && ends_walk(u.class as i32, t.mode as u8, mode))
                || (key.unit_type == MONSTER
                    && mode_changed
                    && t.mode_ends == u.mode_ends
                    && monster_ends_walk(u.class as i32, t.mode as u8, mode))
            {
                p.walk_end = Some((t.mode as u8, t.frame, t.frame_count, t.speed));
            }
            let fresh = skill_start_request(t.mode_requests, u);
            p.skill_request = if is_local && self.click_late {
                // the input layer held it already; a start still held from
                // before goes out first and this pass's start waits one
                // pass (never dropped)
                match t.held_start.take() {
                    Some(held) => {
                        t.held_start = fresh;
                        Some(held)
                    }
                    None => fresh,
                }
            } else if is_local {
                std::mem::replace(&mut t.held_start, fresh)
            } else {
                fresh
            };
            match key.unit_type {
                MONSTER => {
                    p.base_class = rows.base_class(u.class);
                    p.critter = rows.critter(u.class);
                    let (flags, hc) = match &u.kind {
                        KindData::Monster(m) => (m.flags, m.hc_idx),
                        _ => (0, 0),
                    };
                    p.record = rows.record_index(u.class, flags, hc);
                }
                PLAYER if is_local => p.weapon_hit_class = local_weapon.unwrap_or(0),
                ITEM => {
                    let row = item_views
                        .get(&key)
                        .and_then(|v| v.code)
                        .and_then(|c| rows.item_sounds.get(&c).copied());
                    p.item = Some((row, u.mode as u8));
                }
                _ => {}
            }
            // Advance the remembered state (the animation restarts with a
            // new mode).
            if first || mode_changed || restarted {
                if key.unit_type == MONSTER && u.frame_count > 0 && u.mode == mode {
                    // The model's own animation (`bridge::monster_anim`):
                    // the unit's animation rate and, for a monster first
                    // seen, the first frame rolled on its seed.
                    t.frame = u.frame as u32;
                    t.frame_count = u.frame_count as u32;
                    t.speed = u.speed.unwrap_or(0);
                } else {
                    let mut shown = u.clone();
                    shown.mode = mode;
                    let (f, s) = rows.animation(&shown).unwrap_or((0, 0));
                    t.frame = 0;
                    t.frame_count = f;
                    t.speed = if key.unit_type == PLAYER && mode == 18 && u.mode == mode {
                        // A sequence skill (mode 18): the model's +0x4C,
                        // the sequence rate with mode 18's −30
                        // (`bridge::player_anim`). Jab's weapon swing plays
                        // 8 ticks after the start, (6 × 256 + 128) / 204
                        // (audio-cast-jab-ama, one hand thrust05 at T 27).
                        u.speed
                            .unwrap_or_else(|| player_rate(world, key, &shown, s))
                    } else if key.unit_type == PLAYER {
                        player_rate(world, key, &shown, s)
                    } else {
                        s
                    };
                }
            }
            t.mode = mode;
            t.mode_requests = u.mode_requests;
            t.anim_restarts = u.anim_restarts;
            t.mode_changes = u.mode_changes;
            t.mode_ends = u.mode_ends;
            t.states = u.states.clone();
            out.push(p);
        }
        self.tracks.retain(|k, _| live.contains(k));
        out
    }

    /// One audio frame's unit pass: the changes since the last frame at
    /// `cs[0]` (the frame's first client update, or its only one), then
    /// the per-update voices and footsteps for each C in `cs`.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        cx: &mut Ctx,
        world: &ClientWorld,
        position: &dyn Fn(UnitKey) -> Option<(i32, i32)>,
        after_local: &dyn Fn(bool),
        drawn: Option<(UnitKey, u32)>,
        material1: i32,
        updates: &[u32],
        unit_sounds: &mut BTreeMap<(UnitKey, bool), UnitSound>,
    ) -> Result<(), TriggerError> {
        let plan = self.plan(world, position, drawn);
        let Some(rows) = self.rows.clone() else {
            return Ok(());
        };
        let in_town = world
            .player_level()
            .is_some_and(|l| TOWN_LEVELS.contains(&i32::from(l)));
        let c0 = cx.c;
        for p in &plan {
            let cu = if p.client_only {
                world.objclient.set_c.get(&p.key)
            } else {
                world.units.get(&p.key)
            };
            let Some(cu) = cu else {
                continue;
            };
            // The monsters run after the local player and hear its
            // pushed-out point (the push-out ends the player update);
            // objects and items hear the true point
            // (measured).
            after_local(p.key.unit_type == MONSTER);
            let record = p.record.and_then(|r| rows.monsounds.get(r));
            let mode = sound_mode(cu, drawn) as u8;
            let mut u = Unit::new(p.key, cu.class as i32);
            u.base_class = p.base_class;
            u.mode = mode;
            u.is_local = p.is_local;
            u.monsounds = record;
            u.critter = p.critter;
            u.state_146 = p.state_146;
            u.frozen = p.frozen;
            u.dying = p.dying;
            u.weapon_hit_class = p.weapon_hit_class;
            u.in_town = in_town;
            u.near_local = p.near_local;
            u.local_dist = p.local_dist;
            let t = self.tracks.entry((p.key, p.client_only)).or_default();
            u.frame = t.frame;
            u.frame_count = t.frame_count;
            u.speed = t.speed;
            let us = unit_sounds.entry((p.key, p.client_only)).or_default();
            us.hit_class = p.hit_class;
            cx.c = updates.first().copied().unwrap_or(c0);
            // A queued request's sounds wait for the drain after the
            // first update's idle voice (below).
            let deferred = p.queued && !updates.is_empty();
            match p.key.unit_type {
                PLAYER | MONSTER => {
                    if p.key.unit_type == MONSTER && p.first {
                        init_voice(cx, &u, us);
                    }
                    if !deferred {
                        request_sounds(cx, &u, us, p, &rows)?;
                    }
                }
                MISSILE if p.first => {
                    if let Some(id) = rows.missile_travel(cu.class) {
                        cx.unit_request(id, p.key);
                    }
                }
                ITEM => item_sound(cx, p),
                _ => {}
            }
            for &s in &p.states_on {
                let st = rows.states.get(usize::from(s)).copied().unwrap_or_default();
                let dead = cu.is_dead();
                state_on(cx, p.key, false, st.notondead, dead, st.on);
            }
            for &s in &p.states_off {
                let st = rows.states.get(usize::from(s)).copied().unwrap_or_default();
                state_off(cx, p.key, true, st.off);
            }
            if !matches!(p.key.unit_type, PLAYER | MONSTER) {
                continue;
            }
            // Per client update: the sounds of an update read f before
            // that update's step: a player's footstep, a monster's umod
            // hooks and footstep.
            for (n, &c) in updates.iter().enumerate() {
                cx.c = c;
                // The update that ends
                // the walk runs the footstep in the walk mode on the frame
                // before its advance; the arrival sets neutral after it.
                if let (0, Some((wm, wf, wfc, ws))) = (n, p.walk_end) {
                    let mut w = u;
                    w.mode = wm;
                    w.frame = wf;
                    w.frame_count = wfc;
                    w.speed = ws;
                    let us = unit_sounds.entry((p.key, p.client_only)).or_default();
                    let k =
                        footstep_material(material1, floor_under_of(world, p.key, p.client_only));
                    footstep(cx, &w, us, k)?;
                }
                let t = self.tracks.entry((p.key, p.client_only)).or_default();
                let before = t.frame;
                if t.frame_count > 0 {
                    t.frame = (t.frame as i32).wrapping_add(t.speed) as u32;
                    if t.frame >= t.frame_count {
                        t.frame %= t.frame_count;
                    }
                }
                u.frame = before;
                // The umod phase-2 hooks (a monster in a mode below 16):
                // before the idle voice and the footstep, reading f before
                // the update's step. Their client missiles are not
                // modelled here.
                if p.key.unit_type == MONSTER && mode < 16 {
                    if let KindData::Monster(m) = &cu.kind {
                        let (x, y) = cu.cell();
                        let unique = m.flags & 8 != 0;
                        for umod in m.umods {
                            umod_phase2(
                                cx,
                                &u,
                                umod,
                                unique,
                                (i32::from(x), i32::from(y)),
                                &mut |_| {},
                            );
                        }
                    }
                }
                let us = unit_sounds.entry((p.key, p.client_only)).or_default();
                if p.key.unit_type == MONSTER {
                    if deferred && n == 0 {
                        // The update runs before the queue drain: the
                        // idle voice reads the mode before the request.
                        u.mode = p.prev_mode;
                        neutral(cx, &u, us);
                        u.mode = mode;
                        request_sounds(cx, &u, us, p, &rows)?;
                    } else {
                        neutral(cx, &u, us);
                    }
                }
                // A monster's footstep reads f before the update's advance
                // like a player's (the NPC footsteps would otherwise come
                // one update early
                // with the advance first).
                if !skips_footstep(p.key.unit_type, p.first || p.mode_changed || p.restarted, n)
                    && footstep_called(p.key.unit_type, cu.class as i32, mode)
                    && !(p.key.unit_type == PLAYER && cu.class >= 7)
                {
                    let k =
                        footstep_material(material1, floor_under_of(world, p.key, p.client_only));
                    footstep(cx, &u, us, k)?;
                }
            }
        }
        cx.c = c0;
        Ok(())
    }
}

/// The sounds of the mode requests since the last pass: the mode set's
/// mode sound and skill start, else a bare skill
/// start, then the explicit mode sound of a code 0x13 request.
fn request_sounds(
    cx: &mut Ctx,
    u: &Unit,
    us: &mut UnitSound,
    p: &Planned,
    rows: &UnitSoundRows,
) -> Result<(), TriggerError> {
    let start = p.skill_request.and_then(|k| rows.skill_start_row(k));
    // A mode changed and changed back since the last pass sets
    // the mode again.
    if p.mode_changed || p.reentered {
        mode_set(cx, u, us, u.mode, start.as_ref().map(|k| (k, true)))?;
    } else if let Some(k) = start {
        skill_start(cx, u, &k, true)?;
    }
    if let Some(m) = p.explicit {
        mode_sound(cx, u, us, m)?;
    }
    Ok(())
}

/// The floor under a unit for its footstep material: the floor record at
/// the unit's path sub-tile, (x − room x) / 5 and (y − room y) / 5 in
/// truncating division, searched in the unit's room, then in its
/// adjacency array in order; the record's DT1 flags
/// give the material. The path sub-tile is the high word of the path
/// position: the
/// predicted one for the local player, the tracked precise one for a
/// walking monster, else the model cell.
#[cfg(test)]
pub(crate) fn floor_under(world: &ClientWorld, key: UnitKey) -> Floor {
    floor_under_of(world, key, false)
}

/// [`floor_under`] of a set S unit or, `client_only`, a set C one (filed
/// in the room lists under its [`view_key`]).
fn floor_under_of(world: &ClientWorld, key: UnitKey, client_only: bool) -> Floor {
    let (room_key, u) = if client_only {
        (view_key(key), world.objclient.set_c.get(&key))
    } else {
        (key, world.units.get(&key))
    };
    let (Some(room), Some(cd)) = (world.unit_room(room_key), world.drlg.as_ref()) else {
        return Floor::NoRoom;
    };
    let Some(u) = u else {
        return Floor::NoRoom;
    };
    let precise = if world.local_player == Some(key) {
        world.local_position()
    } else {
        u.precise
    };
    let (x, y) = match (precise, u.position) {
        (Some((x, y)), _) => ((x >> 16) as i32, (y >> 16) as i32),
        (None, Some((x, y))) => (i32::from(x), i32::from(y)),
        (None, None) => return Floor::NoRoom,
    };
    let d = &cd.drlg;
    let find = |id| {
        let a = d.active_room(id)?;
        let (c, r) = ((x - a.subtiles.x) / 5, (y - a.subtiles.y) / 5);
        let rec = d
            .room(id)
            .tiles()?
            .floors
            .iter()
            .find(|f| f.x == c && f.y == r)?;
        Some(Floor::Tile(d.tile_info(rec.tile).material))
    };
    find(room.room)
        .or_else(|| cd.adjacency(room.room).into_iter().find_map(find))
        .unwrap_or(Floor::NotFound)
}

/// A monster's first update in a mode (`entered`: first seen, mode
/// changed or animation restarted) makes no footstep call: town walkers
/// start stepping at their second frame crossing, with the reduced first
/// volume, and a unit already walking when first seen makes no step on
/// that first update.
fn skips_footstep(unit_type: u8, entered: bool, update: usize) -> bool {
    unit_type == MONSTER && entered && update == 0
}

/// Whether the local player's change from `old` to `new` is a walk's
/// arrival: a footstep mode
/// into neutral or town neutral; that update still steps in `old`.
fn ends_walk(class: i32, old: u8, new: u32) -> bool {
    class < 7 && matches!(new, 1 | 5) && footstep_called(PLAYER, class, old)
}

/// Whether a monster's change from `old` to `new` is a walk the server
/// stopped: a footstep mode into neutral, for a
/// class that makes no footstep in neutral. The caller adds that the
/// unit's own update ran no mode end (`ClientUnit::mode_ends`): a walk
/// whose path arrived went neutral inside the update, before its footstep
/// call, and makes no step.
fn monster_ends_walk(class: i32, old: u8, new: u32) -> bool {
    new == 1 && footstep_called(MONSTER, class, old) && !footstep_called(MONSTER, class, 1)
}

/// The explicit mode sound of a mode request the pass has not seen: code
/// 0x13 (hit, no mode change) makes the mode sound with m = 0x13 for a
/// player (3 in a were-form)
/// and 0xD for a monster. A unit first seen has no new request.
///
/// The were-form test reads flag-ex bit 3, which the state toggle
/// is kept by the state toggle (`bridge::disguise::toggle`); the player
/// sound passes 3 when it is set.
///
/// D2RUST's own choice: the pass sees only the last request of a frame,
/// so a 0x13 followed by another request before the next audio frame
/// makes no sound (one server tick per frame in play).
fn explicit_mode_sound(u: &ClientUnit, first: bool, seen: u32) -> Option<u8> {
    if first || u.mode_requests == seen {
        return None;
    }
    match (u.key.unit_type, u.last_mode_request) {
        (PLAYER, Some(r)) if r.code == 0x13 => Some(if u.flag_ex & FLAG_EX_DISGUISE != 0 {
            3
        } else {
            0x13
        }),
        (MONSTER, Some(r)) if r.code == 0x13 => Some(0x0D),
        _ => None,
    }
}

/// The mode the unit sounds read: the model's, except the local player's
/// drawn mode while the preview walks it (the client's own path
/// code is not in the model, so the prediction's walk / run mode stands
/// for the client's mode 2 / 3).
fn effective_mode(u: &ClientUnit, drawn: Option<(UnitKey, u32)>) -> u32 {
    match drawn {
        Some((k, m)) if k == u.key => m,
        _ => u.mode,
    }
}

/// The units of the sound pass in the client update's order:
/// set S, then the set C monsters
/// (the critters of the room pass), whose voices, mode sounds and
/// footsteps the same unit sound calls make (a Rogue Encampment chicken's
/// S1 voice
/// `creature_chicken_1`). The other set C units (river objects, client
/// missiles) have their own passes.
fn sound_units(world: &ClientWorld) -> impl Iterator<Item = (UnitKey, &ClientUnit, bool)> {
    let s = world.units.iter().map(|(&k, u)| (k, u, false));
    let c = world
        .objclient
        .set_c
        .iter()
        .filter(|(k, _)| k.unit_type == MONSTER)
        .map(|(&k, u)| (k, u, true));
    s.chain(c)
}

/// The mode the sound triggers read:
/// a player or monster with a running sequence
/// (+0x30 ≠ 0) reads the sequence's drawn mode +0x40, any other unit its
/// mode (with the local player's preview mode of [`effective_mode`]). A
/// Fallen shaman's mode 14 cast is drawn as A2, so its mode set sounds the
/// A2 attack. The mode-change test and the animation
/// track keep the model's mode word: the drawn mode moves frame by frame
/// inside one mode set.
fn sound_mode(u: &ClientUnit, drawn: Option<(UnitKey, u32)>) -> u32 {
    match (&u.sequence, u.key.unit_type) {
        (Some(seq), PLAYER | MONSTER) => seq.drawn_mode,
        _ => effective_mode(u, drawn),
    }
}

/// The item sounds of a mode change: the cursor pickup and the
/// drop (also for a first sight).
fn item_sound(cx: &mut Ctx, p: &Planned) {
    let Some((row, mode)) = p.item else {
        return;
    };
    let entered = p.mode_changed || p.first;
    if !entered {
        return;
    }
    match mode {
        // On the cursor: 235 `item_pickup`, none (a change only).
        4 if p.mode_changed => {
            cx.s.request(235, None, 0, 0, 0);
        }
        // Dropping: 216 `item_flippy`, then the base row's drop sound with
        // its frame delay (0 → 12), volume 180.
        5 => {
            cx.unit_request(216, p.key);
            if let Some(r) = row.filter(|r| r.dropsound > 0) {
                let d = if r.dropsfxframe == 0 {
                    12
                } else {
                    r.dropsfxframe
                };
                let h = cx.s.request(r.dropsound, Some(p.key), d, 0, 0);
                cx.s.set_volume(h, 180);
            }
        }
        _ => {}
    }
}

/// The skill whose start sounds a unit's new mode request plays:
/// the unit's last cast request (code
/// 0x15 / 0x16, record entry 0 the skill) when it is one of the requests
/// since `seen`. The game plays the start inside the cast request itself,
/// so a 0x13 soft hit later in the same tick does not drop it. The local
/// player's own click request (level 0) is the one the start comes from;
/// the server sends the own client no second request, so no request is
/// skipped.
fn skill_start_request(seen: u32, u: &ClientUnit) -> Option<u16> {
    let new = u.mode_requests.wrapping_sub(seen);
    u.last_cast
        .filter(|c| u.mode_requests.wrapping_sub(c.count) < new)
        .and_then(|c| u16::try_from(c.request.record[0]).ok())
}

#[cfg(test)]
mod explicit_tests {
    use super::*;
    use crate::bridge::world::ModeRequest;

    /// A new code 0x13 request is the
    /// explicit mode sound (player m = 0x13, 3 in a were-form, monster
    /// 0xD); a repeat of the count, another code, a unit first seen: none.
    #[test]
    fn a_new_hit_request_is_the_explicit_mode_sound() {
        let mut u = ClientUnit::new(UnitKey::new(PLAYER, 1));
        u.last_mode_request = Some(ModeRequest {
            code: 0x13,
            record: [0; 7],
        });
        u.mode_requests = 3;
        assert_eq!(explicit_mode_sound(&u, false, 2), Some(0x13));
        assert_eq!(explicit_mode_sound(&u, false, 3), None);
        assert_eq!(explicit_mode_sound(&u, true, 2), None);
        let mut m = u.clone();
        m.key = UnitKey::new(MONSTER, 7);
        assert_eq!(explicit_mode_sound(&m, false, 2), Some(0x0D));
        // A player in a were-form (flag-ex bit 3): m = 3.
        let mut w = u.clone();
        w.flag_ex |= FLAG_EX_DISGUISE;
        assert_eq!(explicit_mode_sound(&w, false, 2), Some(3));
        u.last_mode_request = Some(ModeRequest {
            code: 0x14,
            record: [0; 7],
        });
        assert_eq!(explicit_mode_sound(&u, false, 2), None);
    }
}

#[cfg(test)]
mod rate_tests {
    use super::*;

    #[test]
    fn a_players_town_walk_steps_at_213_as_recorded() {
        // The
        // amazon's TW (mode 6, AnimData 256, F 2048) from its mode set:
        // f = 0, 213, 426, 639, …, 1917, then 82 (wrap); TN (mode 5,
        // AnimData speed 80) at 80.
        let key = UnitKey::new(PLAYER, 1);
        let mut u = ClientUnit::new(key);
        u.mode = 6;
        let mut w = ClientWorld::default();
        w.units.insert(key, u.clone());
        let s = player_rate(&w, key, &u, 256);
        assert_eq!(s, 213);
        let (mut f, mut seen) = (0i32, vec![0]);
        for _ in 0..10 {
            f = (f + s) % 2048;
            seen.push(f);
        }
        assert_eq!(&seen[..4], &[0, 213, 426, 639]);
        assert_eq!(&seen[9..], &[1917, 82]);
        u.mode = 5;
        assert_eq!(player_rate(&w, key, &u, 80), 80);
    }
}

#[cfg(test)]
mod sound_mode_tests {
    use super::*;
    use crate::bridge::world::MonsterSequence;

    #[test]
    fn a_unit_in_a_sequence_sounds_its_drawn_mode() {
        // A Fallen shaman 1:45 (class 58) set to mode 14 at frame 163
        // requests its A2 attack voice and weapon swing on that mode set.
        let key = UnitKey::new(MONSTER, 45);
        let mut u = ClientUnit::new(key);
        u.mode = 14;
        assert_eq!(sound_mode(&u, None), 14);
        u.sequence = Some(MonsterSequence {
            frames: Vec::new(),
            length: 17 << 8,
            pos: 0,
            step: 0x100,
            drawn_mode: 5,
        });
        assert_eq!(sound_mode(&u, None), 5);
        assert_eq!(effective_mode(&u, None), 14);
        // The local player's preview mode stands only without a sequence.
        let p = UnitKey::new(PLAYER, 1);
        let mut l = ClientUnit::new(p);
        l.mode = 1;
        assert_eq!(sound_mode(&l, Some((p, 2))), 2);
    }
}

#[cfg(test)]
mod sound_units_tests {
    use super::*;

    #[test]
    fn set_c_monsters_follow_every_set_s_unit() {
        // A set C critter may share its GUID with a set S monster; both
        // are sounded, set S first.
        let mut w = ClientWorld::default();
        let s_mon = UnitKey::new(MONSTER, 5);
        let s_item = UnitKey::new(ITEM, 9);
        w.units.insert(s_mon, ClientUnit::new(s_mon));
        w.units.insert(s_item, ClientUnit::new(s_item));
        let c_mon = UnitKey::new(MONSTER, 5);
        w.objclient.set_c.insert(c_mon, ClientUnit::new(c_mon));
        // A client object is not a unit of this pass (it has its own pass).
        let c_obj = UnitKey::new(2, 6);
        w.objclient.set_c.insert(c_obj, ClientUnit::new(c_obj));
        let got: Vec<_> = sound_units(&w).map(|(k, _, c)| (k, c)).collect();
        assert_eq!(got, vec![(s_mon, false), (s_item, false), (c_mon, true)]);
    }
}

#[cfg(test)]
mod skill_start_tests {
    use super::*;
    use crate::bridge::world::{CastRequest, ModeRequest};

    fn cast(u: &mut ClientUnit, skill: i32) {
        u.mode_requests += 1;
        let request = ModeRequest {
            code: 0x15,
            record: [skill, 1, 5, 5, 0, 0, 0],
        };
        u.last_mode_request = Some(request);
        u.last_cast = Some(CastRequest {
            request,
            count: u.mode_requests,
            late: false,
            tick: 0,
        });
    }

    fn soft_hit(u: &mut ClientUnit) {
        u.mode_requests += 1;
        u.last_mode_request = Some(ModeRequest {
            code: 0x13,
            record: [0; 7],
        });
    }

    #[test]
    fn the_local_players_click_request_at_level_0_plays_its_start() {
        let mut u = ClientUnit::new(UnitKey::new(PLAYER, 1));
        cast(&mut u, 44);
        assert_eq!(skill_start_request(0, &u), Some(44));
        assert_eq!(skill_start_request(1, &u), None);
    }

    /// The game plays the start inside the cast request; a
    /// 0x13 soft hit in the same tick sets no mode and does not undo it.
    /// A cast seen before the hit is not started again.
    #[test]
    fn a_soft_hit_after_a_cast_in_the_same_tick_keeps_its_start() {
        let mut u = ClientUnit::new(UnitKey::new(MONSTER, 1));
        cast(&mut u, 44);
        soft_hit(&mut u);
        assert_eq!(skill_start_request(0, &u), Some(44));
        assert_eq!(skill_start_request(1, &u), None);
        assert_eq!(skill_start_request(2, &u), None);
    }
}

#[cfg(test)]
mod footstep_entry_tests {
    use super::*;

    /// A monster's first update in a mode,
    /// also as a unit first seen, makes no footstep call; its later
    /// updates and every player update do.
    #[test]
    fn a_walk_ending_in_neutral_still_steps_in_the_walk_mode() {
        // fx-firestorm: town walk (6) → town neutral (5), call 54
        assert!(ends_walk(1, 6, 5));
        assert!(ends_walk(1, 2, 1));
        assert!(ends_walk(1, 3, 1));
        assert!(!ends_walk(1, 6, 10)); // a cast is no arrival
        assert!(!ends_walk(1, 5, 1)); // no walk before
        assert!(!ends_walk(7, 6, 5)); // no class footstep row
    }

    #[test]
    fn a_monster_walk_stopped_into_neutral_still_steps_in_the_walk_mode() {
        // gen-aud-quest-countess: corrupt rogue 1:28 (class 45), WL → NU at T 62
        assert!(monster_ends_walk(45, 2, 1));
        assert!(monster_ends_walk(45, 15, 1));
        assert!(!monster_ends_walk(45, 2, 4)); // not neutral
        assert!(!monster_ends_walk(45, 1, 1)); // no walk before
        assert!(!monster_ends_walk(15, 2, 1)); // foulcrow1 steps in neutral too
        assert!(!monster_ends_walk(110, 2, 1)); // vulture1 steps only in mode 8
    }

    #[test]
    fn a_monster_entering_its_mode_skips_its_first_footstep() {
        assert!(skips_footstep(MONSTER, true, 0));
        assert!(!skips_footstep(MONSTER, true, 1));
        assert!(!skips_footstep(MONSTER, false, 0));
        assert!(!skips_footstep(PLAYER, true, 0));
    }
}

#[cfg(test)]
mod queued_request_tests {
    use super::*;

    /// The mode read: a monster's mode request
    /// from the server is drained after its update, so the pass keeps
    /// the mode it had before for that update's idle voice; a mode
    /// change with no new request (the client's own mode end) is not
    /// queued.
    #[test]
    fn a_monsters_server_request_is_queued_behind_its_update() {
        let mut feed = UnitFeed::default();
        feed.set_rows(Arc::new(UnitSoundRows::default()));
        let key = UnitKey::new(MONSTER, 14);
        let mut u = ClientUnit::new(key);
        u.mode = 1;
        let mut w = ClientWorld::default();
        w.units.insert(key, u.clone());
        let at = |_| Some((0, 0));
        let p = feed.plan(&w, &at, None);
        assert!(p[0].first && !p[0].queued);
        u.mode = 5;
        u.mode_requests += 1;
        w.units.insert(key, u.clone());
        let p = feed.plan(&w, &at, None);
        assert!(p[0].mode_changed && p[0].queued);
        assert_eq!(p[0].prev_mode, 1);
        u.mode = 1;
        w.units.insert(key, u.clone());
        let p = feed.plan(&w, &at, None);
        assert!(p[0].mode_changed && !p[0].queued);
        assert_eq!(p[0].prev_mode, 5);
        // the client's mode end to NU, with a server request in the
        // same pass: still the mode end
        u.mode = 4;
        u.mode_requests += 1;
        w.units.insert(key, u.clone());
        assert!(feed.plan(&w, &at, None)[0].queued);
        u.mode = 1;
        u.mode_requests += 1;
        w.units.insert(key, u.clone());
        let p = feed.plan(&w, &at, None);
        assert!(p[0].mode_changed && !p[0].queued);
    }
}
