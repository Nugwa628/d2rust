// D2RUST's own: dev tools.
//! The dev command handlers, shared by the game's own window and the
//! dedicated test server. They are plain functions on a running game
//! ([`Sim`]), never a Bevy system: the caller runs them on the game's
//! thread between two frames, with the character it means named by its
//! player unit. Creation and warps go through the `poke` directives
//! ([`apply_now`]); stat changes through `StatLists::unit_set`, so the
//! game's own update messages carry them.

use std::collections::{BTreeMap, BTreeSet};

use d2_data::bin::TableFiles as _;
use d2_data::tables::{Armor, Levels, Misc, Monstats, Weapons};
use d2_proto::dev::{
    help, DevCommand, DevReply, DevSwitchState, Difficulty, KillRange, Quality, Target,
};
use d2_sim::poke::{Coord, Directive, PokeOp, PokeResult, Spawn, SpawnKind};
use d2_sim::stats::stat;
use d2_sim::units::modes::monster_mode;
use d2_sim::units::{UnitId, UnitType};

use crate::game::{LiveData, Sim};

/// Stats of `combat/vitals`.
mod vstat {
    pub const STATPTS: u16 = 4;
    pub const NEWSKILLS: u16 = 5;
    pub const GOLD: u16 = 14;
    pub const VELOCITYPERCENT: u16 = 67;
}

/// The skill-delay state (`skills` cooldown).
const SKILL_DELAY_STATE: u16 = 121;
/// The town of each act.
const TOWNS: [u32; 5] = d2_sim::world::waypoints::TOWNS;
/// (quest, bit) of the act-opening flags.
const ACT_FLAGS: [(u8, u8); 4] = [(6, 0), (14, 0), (22, 0), (26, 0)];
/// `kill screen` radius in sub-tiles (Chebyshev).
const SCREEN_RADIUS: i32 = 40;

/// The names the handlers resolve, built once from the game's tables.
#[derive(Clone, Debug, Default)]
pub struct DevCatalog {
    /// (level id, `LevelName`).
    pub levels: Vec<(u32, String)>,
    /// The `Act` byte (0-based) of each entry of `levels`.
    pub level_acts: Vec<u8>,
    /// (code, name) of `weapons`, `armor`, `misc` in table order.
    pub items: Vec<(String, String)>,
    /// (monstats row, name).
    pub monsters: Vec<(u32, String)>,
}

/// The first entry whose name is `text` (case-insensitive), else the
/// first whose name contains it.
fn find_name<'a, T>(rows: &'a [T], text: &str, name: impl Fn(&T) -> &str) -> Option<&'a T> {
    let t = text.to_ascii_lowercase();
    rows.iter()
        .find(|r| name(r).eq_ignore_ascii_case(&t))
        .or_else(|| {
            rows.iter()
                .find(|r| name(r).to_ascii_lowercase().contains(&t))
        })
}

impl DevCatalog {
    /// The names the handlers resolve, with the user's own string tables.
    pub fn load(live: &LiveData) -> Result<DevCatalog, String> {
        let strings = d2_data::strings::StringTables::load_from(
            live.archives.as_ref(),
            d2_data::bin::DEFAULT_LANGUAGE,
            live.archives.lod(),
        )
        .map_err(|e| e.to_string())?;
        Ok(Self::from_live(live, |id| {
            strings
                .by_index(u32::from(id))
                .map_or_else(|_| String::new(), |t| String::from_utf16_lossy(&t))
        }))
    }

    /// The names the handlers resolve, walked from the game's tables;
    /// `text` gives the text of a string id the tables name.
    pub fn from_live(live: &LiveData, text: impl Fn(u16) -> String) -> DevCatalog {
        let cstr = |b: &[u8]| {
            let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
            String::from_utf8_lossy(&b[..end]).into_owned()
        };
        let t = &live.tables;
        let (levels, level_acts) = t
            .rows::<Levels>()
            .map(|rows| {
                rows.iter()
                    .enumerate()
                    .skip(1)
                    .map(|(i, r)| ((i as u32, cstr(&r.levelname)), r.act))
                    .unzip()
            })
            .unwrap_or_default();
        let mut items = Vec::new();
        let mut add = |code: &[u8; 4], name: u16| {
            let c = cstr(code).trim().to_string();
            if !c.is_empty() {
                items.push((c, text(name)));
            }
        };
        for r in t.rows::<Weapons>().unwrap_or_default() {
            add(&r.code, r.namestr);
        }
        for r in t.rows::<Armor>().unwrap_or_default() {
            add(&r.code, r.namestr);
        }
        for r in t.rows::<Misc>().unwrap_or_default() {
            add(&r.code, r.namestr);
        }
        let monsters = t
            .rows::<Monstats>()
            .map(|rows| {
                rows.iter()
                    .enumerate()
                    .map(|(i, r)| (i as u32, text(r.namestr)))
                    .collect()
            })
            .unwrap_or_default();
        DevCatalog {
            levels,
            level_acts,
            items,
            monsters,
        }
    }

    /// A level id by id or name.
    pub fn level(&self, t: &Target) -> Result<(u32, String), String> {
        match t {
            Target::Id(n) => self
                .levels
                .iter()
                .find(|(id, _)| id == n)
                .cloned()
                .ok_or_else(|| format!("no level {n}")),
            Target::Name(s) => find_name(&self.levels, s, |(_, n)| n)
                .cloned()
                .ok_or_else(|| format!("no level named {s:?}")),
        }
    }

    /// An item code by code or name.
    pub fn item(&self, t: &Target) -> Result<(String, String), String> {
        let Target::Name(s) = t else {
            return Err("item: give a code or a name".into());
        };
        if let Some(r) = self
            .items
            .iter()
            .find(|(c, _)| s.len() <= 4 && c.eq_ignore_ascii_case(s))
        {
            return Ok(r.clone());
        }
        find_name(&self.items, s, |(_, n)| n)
            .cloned()
            .ok_or_else(|| format!("no item code or name {s:?}"))
    }

    /// A monstats row by id or name.
    pub fn monster(&self, t: &Target) -> Result<(u32, String), String> {
        match t {
            Target::Id(n) => Ok((
                *n,
                self.monsters
                    .iter()
                    .find(|(id, _)| id == n)
                    .map_or_else(String::new, |(_, s)| s.clone()),
            )),
            Target::Name(s) => find_name(&self.monsters, s, |(_, n)| n)
                .cloned()
                .ok_or_else(|| format!("no monster named {s:?}")),
        }
    }
}

/// The handlers' state for one game.
#[derive(Clone, Debug, Default)]
pub struct DevServer {
    pub catalog: DevCatalog,
    /// Each player's base velocity percent before `speed` changed it.
    speed_base: BTreeMap<UnitId, i32>,
}

impl DevServer {
    pub fn new(catalog: DevCatalog) -> Self {
        Self {
            catalog,
            speed_base: BTreeMap::new(),
        }
    }

    /// Runs one command on the game now for `player` (`None`: the
    /// character's player doesn't exist yet, so only the commands that
    /// need no character work); the reply carries the switch states
    /// after it.
    pub fn apply(&mut self, s: &mut Sim, player: Option<UnitId>, cmd: &DevCommand) -> DevReply {
        let mut r = match self.run(s, player, cmd) {
            Ok(text) => DevReply::ok(text),
            Err(text) => DevReply::error(text),
        };
        r.switches = player.map_or_else(DevSwitchState::default, |p| switches(s, p));
        r
    }

    fn run(
        &mut self,
        s: &mut Sim,
        player: Option<UnitId>,
        cmd: &DevCommand,
    ) -> Result<String, String> {
        if let DevCommand::Help(t) = cmd {
            return help(t.as_deref());
        }
        if let DevCommand::Difficulty(d) = cmd {
            return Err(difficulty_hint(*d));
        }
        let p = player.ok_or("not in game yet")?;
        match cmd {
            DevCommand::Help(_) | DevCommand::Difficulty(_) => unreachable!(),
            DevCommand::God(on) => {
                let dev = &mut s.events.action.sys.hooks.dev;
                let on = on.unwrap_or(!dev.is_god(p));
                if on {
                    dev.god.insert(p);
                    heal(s, p);
                } else {
                    dev.god.remove(&p);
                }
                Ok(format!("god {}", on_off(on)))
            }
            DevCommand::NoCooldown(on) => {
                let dev = &mut s.events.action.sys.hooks.dev;
                dev.nocd = on.unwrap_or(!dev.nocd);
                Ok(format!("nocd {}", on_off(dev.nocd)))
            }
            DevCommand::FreeMana(on) => {
                let dev = &mut s.events.action.sys.hooks.dev;
                dev.freemana = on.unwrap_or(!dev.freemana);
                Ok(format!("freemana {}", on_off(dev.freemana)))
            }
            DevCommand::Heal => {
                heal(s, p);
                Ok("healed".into())
            }
            DevCommand::Level(n) => set_level(s, p, *n),
            DevCommand::LevelAdd(n) => {
                let to = (base(s, p, stat::LEVEL) as u32).saturating_add(*n).min(99);
                set_level(s, p, to)
            }
            DevCommand::Gold(n) => {
                let level = base(s, p, stat::LEVEL).max(1);
                let cap = level.saturating_mul(10_000);
                let have = base(s, p, vstat::GOLD);
                let new = have.saturating_add(*n as i32).min(cap);
                set(s, p, vstat::GOLD, new);
                Ok(format!("gold {have} -> {new} (limit {cap})"))
            }
            DevCommand::StatPoints(n) => {
                let v = base(s, p, vstat::STATPTS).saturating_add(*n as i32);
                set(s, p, vstat::STATPTS, v);
                Ok(format!("{v} stat points to spend"))
            }
            DevCommand::SkillPoints(n) => {
                let v = base(s, p, vstat::NEWSKILLS).saturating_add(*n as i32);
                set(s, p, vstat::NEWSKILLS, v);
                Ok(format!("{v} skill points to spend"))
            }
            DevCommand::Town(act) => {
                let level = TOWNS[usize::from(act - 1)];
                warp(s, p, level).map(|()| format!("warped to act {act} town (level {level})"))
            }
            DevCommand::Warp(t) => {
                let (level, name) = self.catalog.level(t)?;
                warp(s, p, level).map(|()| format!("warped to {name} (level {level})"))
            }
            DevCommand::Waypoints => waypoints(s, p),
            DevCommand::Acts => acts(s, p),
            DevCommand::Quest { slot, bit, on } => quest(s, p, *slot, *bit, *on),
            DevCommand::Item {
                item,
                quality,
                ilvl,
                ground,
            } => {
                let (code, name) = self.catalog.item(item)?;
                let level = base(s, p, stat::LEVEL).clamp(1, 99) as u8;
                let ilvl = ilvl.unwrap_or(level);
                let q = quality_name(*quality);
                if !*ground {
                    // No cell fits: the ground (edge case 2).
                    if give_item(s, p, &code, *quality as u8, ilvl)? {
                        return Ok(format!("{q} {name} [{code}] ilvl {ilvl} in the inventory"));
                    }
                }
                let op = PokeOp::Directive(Directive::Item {
                    code: code.clone(),
                    x: Coord::X(0),
                    y: Coord::Y(0),
                    quality: Some(*quality as u8),
                    ilvl: Some(ilvl),
                });
                result(apply_now(s, p, &op))
                    .map(|_| format!("{q} {name} [{code}] ilvl {ilvl} at your feet"))
            }
            DevCommand::Identify => identify(s, p),
            DevCommand::Kill(range) => kill(s, p, *range),
            DevCommand::Spawn { monster, count } => {
                let (class, name) = self.catalog.monster(monster)?;
                let mut made = 0;
                let mut last = String::new();
                for k in 0..i32::from(*count) {
                    // Around the player, 4 sub-tiles out, one per direction.
                    let (dx, dy) =
                        [(4, 0), (0, 4), (-4, 0), (0, -4), (4, 4), (-4, -4)][k as usize % 6];
                    let op = PokeOp::Spawn(Spawn {
                        class,
                        x: Coord::X(dx + k / 6),
                        y: Coord::Y(dy),
                        kind: SpawnKind::Normal,
                        free: true,
                        umods: Vec::new(),
                    });
                    match result(apply_now(s, p, &op)) {
                        Ok(_) => made += 1,
                        Err(e) => last = e,
                    }
                }
                if made == 0 {
                    Err(format!("spawn {name} ({class}): {last}"))
                } else {
                    Ok(format!("spawned {made} x {name} ({class})"))
                }
            }
            DevCommand::Reveal => {
                Err("reveal: not built yet (specs/tools/dev-tools.md open question 2)".into())
            }
            DevCommand::Speed(pct) => {
                let b = *self
                    .speed_base
                    .entry(p)
                    .or_insert_with(|| base(s, p, vstat::VELOCITYPERCENT));
                set(s, p, vstat::VELOCITYPERCENT, b + *pct as i32);
                s.events.action.sys.hooks.dev.speed = *pct;
                Ok(format!("speed +{pct}%"))
            }
        }
    }
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn quality_name(q: Quality) -> &'static str {
    q.name()
}

fn difficulty_hint(d: Difficulty) -> String {
    format!(
        "difficulty is fixed when a game starts: Save and Exit, then start with \
         --difficulty {} (with --save <your .d2s>)",
        d.name()
    )
}

/// A poke result as the reply's.
pub fn result(r: PokeResult) -> Result<Option<u32>, String> {
    match r {
        PokeResult::Ok(g) => Ok(g),
        PokeResult::Failed => Err("the game refused it (no room or no free cell)".into()),
        PokeResult::FailedWith(w) | PokeResult::Gap(w) => Err(w),
        PokeResult::Unresolved(u) => Err(format!("unresolved {u}")),
        PokeResult::Pending => Err("pending".into()),
    }
}

/// `player`'s switch states.
pub fn switches(s: &Sim, player: UnitId) -> DevSwitchState {
    let dev = &s.events.action.sys.hooks.dev;
    DevSwitchState {
        god: dev.is_god(player),
        nocd: dev.nocd,
        freemana: dev.freemana,
        speed: dev.speed,
    }
}

fn base(s: &Sim, u: UnitId, st: u16) -> i32 {
    s.events.action.sys.stats.unit_base(u, st, 0)
}

fn set(s: &mut Sim, u: UnitId, st: u16, v: i32) {
    let sys = &mut s.events.action.sys;
    sys.stats.unit_set(&mut sys.hooks, u, st, v, 0);
}

/// Life, mana and stamina to their maximum.
fn heal(s: &mut Sim, u: UnitId) {
    let st = &s.events.action.sys.stats;
    let (l, m, a) = (st.max_life(u), st.max_mana(u), st.max_stamina(u));
    if base(s, u, stat::HITPOINTS) != l {
        set(s, u, stat::HITPOINTS, l);
    }
    if base(s, u, stat::MANA) != m {
        set(s, u, stat::MANA, m);
    }
    if base(s, u, stat::STAMINA) != a {
        set(s, u, stat::STAMINA, a);
    }
}

/// The upkeep before every tick, for `player`.
pub fn upkeep(s: &mut Sim, player: UnitId) {
    let dev = s.events.action.sys.hooks.dev.clone();
    if dev.is_god(player) {
        heal(s, player);
    } else if dev.freemana {
        let m = s.events.action.sys.stats.max_mana(player);
        if base(s, player, stat::MANA) != m {
            set(s, player, stat::MANA, m);
        }
    }
    if dev.nocd
        && s.events
            .action
            .sys
            .stats
            .unit_list(player)
            .is_some_and(|_| has_state(s, player, SKILL_DELAY_STATE))
    {
        let game = &mut s.game;
        s.events.action.with(game, |g, v| {
            v.set_state(player, SKILL_DELAY_STATE, false);
            let _ = g.lists.queue_update(player);
        });
    }
}

fn has_state(s: &Sim, u: UnitId, state: u16) -> bool {
    s.events.action.sys.stats.has_state(u, u32::from(state))
}

/// `level n`: the experience add to the threshold of level n (Rule 5).
fn set_level(s: &mut Sim, p: UnitId, n: u32) -> Result<String, String> {
    let now = base(s, p, stat::LEVEL) as u32;
    if n < now {
        return Err(format!("level: already level {now} (only raising)"));
    }
    if n == now {
        return Ok(format!("already level {now}"));
    }
    let game = &mut s.game;
    s.events.action.combat(game, |cv, _| {
        let Some(t) = cv.v.h.vitals.clone() else {
            return Err("level: no vitals tables".to_string());
        };
        d2_sim::combat::vitals::set_experience_for_target_level(cv, &t, p, n - 1);
        Ok(())
    })?;
    let got = base(s, p, stat::LEVEL);
    Ok(format!(
        "level {now} -> {got}; {} stat / {} skill points to spend",
        base(s, p, vstat::STATPTS),
        base(s, p, vstat::NEWSKILLS)
    ))
}

fn warp(s: &mut Sim, p: UnitId, level: u32) -> Result<(), String> {
    let op = PokeOp::Directive(Directive::Warp { level, tile: None });
    result(apply_now(s, p, &op)).map(|_| ())
}

/// Every waypoint of every difficulty known.
fn waypoints(s: &mut Sim, p: UnitId) -> Result<String, String> {
    let map = s
        .world
        .action
        .waypoints
        .as_ref()
        .map(|w| w.map.clone())
        .ok_or("waypoints: no waypoint tables")?;
    let idx: BTreeSet<u8> = (1..map.level_count())
        .filter_map(|l| map.index_of_level(l))
        .collect();
    // A new character has no record until its first waypoint: made empty
    // here, as the waypoint code makes it on first use.
    let records = s.events.action.sys.hooks.waypoints.entry(p).or_default();
    for d in 0..3 {
        let r = records.get_mut(d);
        for &n in &idx {
            r.set(u32::from(n))
                .map_err(|e| format!("waypoint {n}: {e}"))?;
        }
    }
    Ok(format!("{} waypoints known in every difficulty", idx.len()))
}

/// The act-opening quest flags.
fn acts(s: &mut Sim, p: UnitId) -> Result<String, String> {
    let q = s
        .world
        .rest
        .quests
        .get_mut(&p)
        .ok_or("acts: the player has no quest record")?;
    for f in &mut q.flags {
        for (quest, bit) in ACT_FLAGS {
            f.set(quest, bit);
        }
    }
    Ok("acts 2-5 open in every difficulty (use waypoints to travel)".into())
}

/// `quest`: one quest flag bit, set or cleared in every difficulty.
fn quest(s: &mut Sim, p: UnitId, slot: u8, bit: u8, on: bool) -> Result<String, String> {
    let q = s
        .world
        .rest
        .quests
        .get_mut(&p)
        .ok_or("quest: the player has no quest record")?;
    for f in &mut q.flags {
        if on {
            f.set(slot, bit);
        } else {
            f.clear(slot, bit);
        }
    }
    let state = if on { "set" } else { "cleared" };
    Ok(format!(
        "quest {slot} bit {bit} {state} in every difficulty"
    ))
}

/// The item code as the tables key it (4 bytes, space-padded).
fn code_key(code: &str) -> Result<[u8; 4], String> {
    let b = code.as_bytes();
    if b.is_empty() || b.len() > 4 {
        return Err(format!("item code {code:?}: 1-4 characters"));
    }
    let mut key = [b' '; 4];
    key[..b.len()].copy_from_slice(b);
    Ok(key)
}

/// `item` into the inventory: the quest reward's creation and placement (spawn
/// mode 4, identified, placed by the inventory model). `Ok(false)`: no cell
/// fits (the item is freed; the caller drops one on the ground instead). The
/// inventory model's messages go with the next tick's sends.
fn give_item(s: &mut Sim, p: UnitId, code: &str, quality: u8, ilvl: u8) -> Result<bool, String> {
    use d2_sim::wiring::economy::quest_reward::{create_reward, place_reward, Placed, QuestInv};
    let key = code_key(code)?;
    let r = s
        .world
        .with_economy(&mut s.game, &mut s.events, |econ, parts| {
            let Some(inv) = parts.inventory.as_deref_mut() else {
                return Err("item: no inventory model in this game".to_string());
            };
            let item = create_reward(econ, p, key, i32::from(ilvl), quality)
                .map_err(|e| format!("item: {e}"))?
                .ok_or("item: the game refused to create it")?;
            let mut q = QuestInv::new(&inv.tables, &mut inv.state, inv.rest.as_mut());
            match place_reward(econ, &mut q, p, item) {
                Placed::Stored(_) => Ok(true),
                Placed::Refused(item) => {
                    econ.free_item(item).map_err(|e| format!("item: {e}"))?;
                    Ok(false)
                }
            }
        });
    let sent = s.world.take_inventory_sent(&mut s.game, &mut s.events);
    s.world.rest.sent.extend(sent);
    r
}

/// `identify`: every item the player holds, as Cain's identify marks one
/// (`InvDesk::identify_unit`).
fn identify(s: &mut Sim, p: UnitId) -> Result<String, String> {
    use d2_sim::wiring::inventory::InvDesk;
    let n = s
        .world
        .with_economy(&mut s.game, &mut s.events, |econ, parts| {
            let Some(inv) = parts.inventory.as_deref_mut() else {
                return Err("identify: no inventory model in this game".to_string());
            };
            let items = inv.state.items_of(p);
            let mut d = InvDesk::new(econ, &inv.tables, &mut inv.state, inv.rest.as_mut());
            Ok(items.into_iter().filter(|&i| d.identify_unit(p, i)).count())
        })?;
    let sent = s.world.take_inventory_sent(&mut s.game, &mut s.events);
    s.world.rest.sent.extend(sent);
    Ok(format!("identified {n} items"))
}

/// `kill`: the game's kill for each living hostile monster in range.
fn kill(s: &mut Sim, p: UnitId, range: KillRange) -> Result<String, String> {
    let hooks = &s.events.action.sys.hooks;
    let level_of = |u: UnitId| {
        s.game
            .lists
            .unit(u)
            .and_then(|e| e.room())
            .and_then(|r| hooks.drlg.level_id(&s.game, r))
    };
    let here = level_of(p).ok_or("kill: the player is in no room")?;
    let (px, py) = hooks.path_position(p);
    let pets: BTreeSet<i32> = hooks
        .pet_lists
        .values()
        .flat_map(|l| l.entries.iter())
        .flat_map(|e| e.nodes.iter().map(|n| n.guid))
        .collect();
    let t = hooks.tables.clone();
    let targets: Vec<UnitId> = s
        .game
        .lists
        .units_of_type(UnitType::Monster)
        .into_iter()
        .filter(|&u| {
            let Some(r) = s.events.action.sys.units.get(u) else {
                return false;
            };
            if r.mode == monster_mode::DT || r.mode == monster_mode::DD {
                return false;
            }
            let row = t.combat.monstats.get(r.class as usize);
            if row.is_none_or(|m| m.npc || m.interact || !m.killable) {
                return false;
            }
            let guid = s.game.lists.unit(u).map_or(-1, |e| e.guid as i32);
            if pets.contains(&guid) || hooks.hireling_units.contains(&u) {
                return false;
            }
            match range {
                KillRange::Level => level_of(u) == Some(here),
                KillRange::Screen => {
                    let (x, y) = hooks.path_position(u);
                    (x - px).abs() <= SCREEN_RADIUS && (y - py).abs() <= SCREEN_RADIUS
                }
            }
        })
        .collect();
    let n = targets.len();
    let game = &mut s.game;
    s.events.action.combat(game, |cv, _| {
        for u in targets {
            d2_sim::wiring::action::reaction::kill_by(cv, u, Some(p));
        }
    });
    Ok(format!("killed {n} monsters"))
}

/// The `objects` rows with operate function 23 (`@wp`).
pub fn waypoint_classes(s: &Sim) -> BTreeSet<u32> {
    s.world
        .action
        .waypoints
        .as_ref()
        .map(|w| {
            w.objects
                .iter()
                .enumerate()
                .filter(|(_, o)| o.operate_fn == 23)
                .map(|(i, _)| i as u32)
                .collect()
        })
        .unwrap_or_default()
}

/// Runs `op` on `s` now for `player`: the player is `@player`,
/// `@wp` and `item` use the game's own tables.
pub fn apply_now(s: &mut Sim, player: UnitId, op: &PokeOp) -> PokeResult {
    let waypoints = waypoint_classes(s);
    // A copy: the loan below takes the world's tables for its call.
    let tables = s.world.tables.clone();
    let env = d2_sim::poke::Env {
        player,
        waypoint_classes: &waypoints,
        items: Some(&tables),
    };
    // The quest parts are lent as in the tick and the 0x13 / waypoint
    // handlers, so a quest object a directive creates (a `warp` that
    // builds an act) runs its init.
    let game = &mut s.game;
    let r = s.world.lend_quests(&mut s.events, |_, ev| {
        d2_sim::poke::apply_op(game, ev, &env, op)
    });
    // The unit work the directive queued (a `warp`'s pet follow) runs before
    // the next tick, as inside the game function the poke calls.
    s.world.handler_work(&mut s.game, &mut s.events);
    r
}
