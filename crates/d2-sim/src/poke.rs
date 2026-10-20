//! Pokes: directives that set game state directly at a chosen tick, for checks that should
//! not walk through the game to get there. This module is a debug and test entry point:
//! nothing in normal play calls it. It does no I/O and draws only what the creation path it
//! calls draws.
//!
//! | Item | Does |
//! |---|---|
//! | [`Directive`], [`parse_directive`], `Display` | one directive: strict parser and canonical writer |
//! | [`Spawn`] | the `spawn` line of a poke file |
//! | [`PokeFile`] | a `poke 1` file: `at <tick> <directive>` lines |
//! | [`apply`], [`apply_op`], [`apply_line`] | resolve the references on the current state and run the directive on a [`WorldSim`] game |
//! | [`spawn_monster`] | the creation calls of a `spawn` step |
//! | [`GotoTarget`], [`GotoWalk`], [`goto_step`] | the `goto` walk, one step per tick |
//! | [`msg_values`] | `msg`: its values with the references resolved (the message bytes are the host side's, `d2-client::app::poke`) |
//!
//! References (`@player`, `@x±N`, `@<type>:<class>#n` and the like) are resolved by
//! [`apply`] on the state it is called on. Callers call it between ticks t − 1 and t, so
//! that is the state after tick t − 1.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::drlg::DrlgRoomId;
use crate::game::Game;
use crate::items::{ItemGame, ItemRequest, ItemTables};
use crate::missiles::param_flags as pf;
use crate::missiles::{create_missile, MissileParams};
use crate::monsters::init::{self, InitHost as _};
use crate::monsters::population::{placement, preset, spawn as pop_spawn};
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::economy::{Economy, GameFields, ItemSpawn};
use crate::wiring::path::act_change;
use crate::wiring::path::place::{level_warp, place_unit};
use crate::wiring::path::PathCtx;
use crate::wiring::worldgen::dispatch::WorldSim;
use crate::wiring::worldgen::WorldPending;

/// Poke file format version, written on the first line as `poke 1`.
pub const POKE_FORMAT_VERSION: u32 = 1;

/// Largest tick a poke file line may name.
pub const MAX_TICK: u32 = 1_000_000;

/// Item qualities by name; the position in the array plus one is the quality
/// value, 1 low … 9 tempered.
pub const QUALITIES: [&str; 9] = [
    "low", "normal", "superior", "magic", "set", "rare", "unique", "crafted", "tempered",
];

/// A coordinate argument: absolute sub-tiles, or the player's position
/// ± N (`@x±N` / `@y±N`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coord {
    /// An absolute sub-tile coordinate.
    Num(i32),
    /// The player's x plus the offset.
    X(i32),
    /// The player's y plus the offset.
    Y(i32),
}

/// A unit argument: a reference resolved against the current unit lists, or a type and GUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitArg {
    /// `@player`.
    Player,
    /// `@<type>[:<class>][#n]`: the n-th unit (ascending GUID) of the
    /// type, of that class when given.
    Nth { ty: u8, class: Option<u32>, n: u32 },
    /// `@wp[#n]`: the n-th waypoint object.
    Waypoint(u32),
    /// `<type>/<guid>`.
    Guid { ty: u8, guid: u32 },
    /// `@su<row>`: the monster spawned as that `superuniques` row.
    Superunique(u16),
}

/// A value of `msg`: a number, the player's position ± N, or a unit (its GUID).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgArg {
    /// A plain number.
    Num(u32),
    /// `@x±N` / `@y±N` (never [`Coord::Num`]).
    Pos(Coord),
    /// A unit, sent as its GUID.
    Unit(UnitArg),
}

/// One directive. Optional arguments are `None` when not written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Directive {
    /// Create the object of `objects` row `class` at (x, y), in `mode` (default 0).
    Object {
        class: u32,
        x: Coord,
        y: Coord,
        mode: Option<u32>,
    },
    Superunique {
        row: u32,
        x: Coord,
        y: Coord,
    },
    /// Create a missile of `class` at (x, y) flying to the absolute target (tx, ty). The
    /// owner defaults to the player and the skill to (0, 0).
    Missile {
        class: u32,
        x: Coord,
        y: Coord,
        tx: Coord,
        ty: Coord,
        /// (skill id, level).
        skill: Option<(u32, u32)>,
        owner: Option<UnitArg>,
    },
    /// Set the game seed to `{lo, hi}`.
    SeedGame {
        lo: u32,
        hi: u32,
    },
    /// Set the seed of a unit to `{lo, hi}`.
    SeedUnit {
        unit: UnitArg,
        lo: u32,
        hi: u32,
    },
    /// Set the environment clock (`period` and `ticks`) of the player's act.
    Time {
        period: u32,
        ticks: u32,
    },
    /// Teleport a unit to (x, y). With `free`, it lands on the nearest free,
    /// missile-passable cell (see [`LAND_MASK`]) instead of the raw point.
    Pos {
        unit: UnitArg,
        x: Coord,
        y: Coord,
        free: bool,
    },
    /// D2RUST's own test aid: one move of at most
    /// [`HOP`] sub-tiles per axis toward (x, y), the first free spot of
    /// [`hop_candidates`] from the unit's current position.
    Hop {
        unit: UnitArg,
        x: Coord,
        y: Coord,
    },
    /// Warp the player to `level`, arriving at warp tile `tile` (default 0).
    Warp {
        level: u32,
        tile: Option<u32>,
    },
    /// Create a ground item with the item code `code` at (x, y).
    Item {
        /// 3–4 characters `[a-z0-9]`.
        code: String,
        x: Coord,
        y: Coord,
        /// 1 low … 9 tempered ([`QUALITIES`]).
        quality: Option<u8>,
        ilvl: Option<u8>,
    },
    /// Set stat `stat` (at `layer`) of a unit to `value`.
    Stat {
        unit: UnitArg,
        stat: u16,
        layer: u16,
        value: i32,
    },
    /// Switch state `state` of a unit on or off.
    State {
        unit: UnitArg,
        state: u16,
        on: bool,
    },
    /// A pause of up to 3600 seconds. `apply` itself changes nothing for it.
    Freeze {
        seconds: u32,
    },
    /// Walk the player to a target unit; run one step per tick through `goto_step`.
    Goto(GotoTarget),
    /// One C→S game message through the local client's sender: the id's fields in layout
    /// order.
    Msg {
        id: u8,
        args: Vec<MsgArg>,
    },
    /// Interact with a unit: C→S 0x13 with its type and GUID, run by the server's
    /// dispatcher immediately.
    Operate {
        unit: UnitArg,
    },
    /// Talk to an NPC: C→S 0x13 and 0x2F, then one message per choice, each run by the
    /// server's dispatcher immediately, in order.
    Talk {
        npc: UnitArg,
        choices: Vec<TalkChoice>,
    },
}

/// One menu choice of `talk`: the C→S message it sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TalkChoice {
    /// `trade`: 0x38 action 1.
    Trade,
    /// `gamble`: 0x38 action 2.
    Gamble,
    /// `hire`: 0x38 action 3, item field the player's GUID.
    Hire,
    /// `action:<n>`: 0x38 action n, item field 0.
    Action(u32),
    /// `quest:<n>`: 0x31 with message n.
    Quest(u16),
    /// `close`: 0x30.
    Close,
}

impl TalkChoice {
    fn parse(t: &str) -> Result<Self, String> {
        Ok(match t {
            "trade" => Self::Trade,
            "gamble" => Self::Gamble,
            "hire" => Self::Hire,
            "close" => Self::Close,
            _ => match t.split_once(':') {
                Some(("action", n)) => Self::Action(num(n)?),
                Some(("quest", n)) => Self::Quest(ranged(n, 0, 0xFFFF, "quest message")? as u16),
                _ => {
                    return Err(format!(
                        "talk choice {t:?}: trade, gamble, hire, action:<n>, quest:<n> or close"
                    ))
                }
            },
        })
    }
}

impl fmt::Display for TalkChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trade => write!(f, "trade"),
            Self::Gamble => write!(f, "gamble"),
            Self::Hire => write!(f, "hire"),
            Self::Action(n) => write!(f, "action:{n}"),
            Self::Quest(n) => write!(f, "quest:{n}"),
            Self::Close => write!(f, "close"),
        }
    }
}

/// One server handler call of `operate` / `talk`: a C→S message id and its field values in
/// the field order of that message's layout; the host side builds the bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerCall {
    /// The C→S message id.
    pub id: u8,
    /// The field values, in layout order.
    pub values: Vec<u32>,
}

/// The target of a `goto`: a unit class, optionally in another level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GotoTarget {
    /// `preset <level>`: the goal level; `None` for `unit` (the player's
    /// level at the first step).
    pub preset: Option<u32>,
    /// `to <level>`: only a tile whose room's warp link leads to this level
    /// (an interaction tile whose lvlwarp class is shared by several links).
    pub to: Option<u32>,
    /// Unit type: 1 monster or 2 object.
    pub ty: u8,
    /// Class (`monstats` / `objects` row).
    pub class: u32,
    /// `superunique <row>`: only the monster spawned as that `superuniques`
    /// row (its minions and ordinary monsters often share its class).
    pub superunique: Option<u16>,
    /// `mode <m>`: only a unit in that mode (a door case asks for a closed
    /// door, mode 0: a door whose class has `PreOperate` may spawn open).
    pub mode: Option<u8>,
    /// `opening`: for a door, land in front of its opening instead of next
    /// to its position ([`door_opening`]).
    pub opening: bool,
}

/// Steps after which a `goto` walk ends `failed`.
pub const GOTO_MAX_STEPS: u32 = 400;

/// The `failed` note of a `goto` that searched every room of its goal level
/// without finding the target, followed by the level id.
pub const GOTO_NONE_IN_LEVEL: &str = "none in level";

/// The state a runner keeps between the steps of one `goto`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GotoWalk {
    /// The goal level, fixed at the first step.
    pub goal: Option<u32>,
    /// DRLG rooms seen: (level id, tile x, tile y).
    pub seen: BTreeSet<(u32, i32, i32)>,
    /// DRLG rooms the player cannot stand in (no free cell, or the
    /// placement refused or landed elsewhere): the search avoids them.
    pub blocked: BTreeSet<(u32, i32, i32)>,
    /// The blocked rooms that could not be loaded at all: their units were
    /// never searched.
    pub unloaded: BTreeSet<(u32, i32, i32)>,
    /// Steps run so far.
    pub steps: u32,
    /// The whole goal level was streamed and populated once, after the
    /// rooms-near search ran out: some levels (the Arcane Sanctuary's
    /// platforms) have no near link to the room the boss stands in.
    pub swept: bool,
}

/// The directive keywords, in the order of the `Directive` variants.
pub const KEYWORDS: [&str; 17] = [
    "object",
    "superunique",
    "missile",
    "seed-game",
    "seed-unit",
    "time",
    "pos",
    "hop",
    "warp",
    "item",
    "stat",
    "state",
    "freeze",
    "goto",
    "msg",
    "operate",
    "talk",
];

impl Directive {
    /// The directive's keyword as written in a poke line.
    pub fn keyword(&self) -> &'static str {
        match self {
            Self::Object { .. } => "object",
            Self::Superunique { .. } => "superunique",
            Self::Missile { .. } => "missile",
            Self::SeedGame { .. } => "seed-game",
            Self::SeedUnit { .. } => "seed-unit",
            Self::Time { .. } => "time",
            Self::Pos { .. } => "pos",
            Self::Hop { .. } => "hop",
            Self::Warp { .. } => "warp",
            Self::Item { .. } => "item",
            Self::Stat { .. } => "stat",
            Self::State { .. } => "state",
            Self::Freeze { .. } => "freeze",
            Self::Goto(_) => "goto",
            Self::Msg { .. } => "msg",
            Self::Operate { .. } => "operate",
            Self::Talk { .. } => "talk",
        }
    }
}

/// The monster kinds of a `spawn` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnKind {
    /// An ordinary monster placed at the point.
    Normal,
    /// A random boss with its champion minions.
    RandomBoss,
    /// A champion pack member with the given unique modifier, plus minions.
    Champion,
    /// A unique boss with the given unique modifiers, plus minions.
    Unique,
}

impl SpawnKind {
    /// Every kind, in declaration order.
    pub const ALL: [SpawnKind; 4] = [Self::Normal, Self::RandomBoss, Self::Champion, Self::Unique];
    /// The kind's name as written in a poke line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::RandomBoss => "random-boss",
            Self::Champion => "champion",
            Self::Unique => "unique",
        }
    }
}

/// `spawn <class> <x> <y> <kind> [free] [umod <id>...]`: a monster spawned through the
/// population path. With `free`, the normal kind searches the population rings when the
/// exact point is blocked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawn {
    /// monstats row.
    pub class: u32,
    /// The x coordinate.
    pub x: Coord,
    /// The y coordinate.
    pub y: Coord,
    /// The monster kind.
    pub kind: SpawnKind,
    /// Search a free point near (x, y) instead of requiring it.
    pub free: bool,
    /// Unique modifier ids (one for a champion, any number for a unique).
    pub umods: Vec<u8>,
}

/// What one poke file line runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PokeOp {
    /// A directive line.
    Directive(Directive),
    /// A `spawn` line.
    Spawn(Spawn),
}

/// One `at <tick> ...` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PokeLine {
    /// The tick, relative to the start of the poke file's run.
    pub tick: u32,
    /// What the line runs.
    pub op: PokeOp,
}

/// A parsed poke file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PokeFile {
    /// Non-decreasing ticks, file order within a tick.
    pub lines: Vec<PokeLine>,
}

/// A poke file line that does not parse.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct PokeError {
    /// 1-based line number (0: the file as a whole).
    pub line: usize,
    /// What is wrong with the line.
    pub message: String,
}

// ---- numbers and references ----------------------------------------------

fn num(t: &str) -> Result<u32, String> {
    let r = match t.strip_prefix("0x") {
        Some(h) if !h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit()) => {
            u32::from_str_radix(h, 16)
        }
        Some(_) => return Err(format!("bad number {t:?}")),
        None if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) => t.parse(),
        None => return Err(format!("bad number {t:?}")),
    };
    r.map_err(|_| format!("number {t:?} does not fit 32 bits"))
}

fn ranged(t: &str, lo: u32, hi: u32, what: &str) -> Result<u32, String> {
    let v = num(t)?;
    if v < lo || v > hi {
        return Err(format!("{what} {v} outside {lo}..={hi}"));
    }
    Ok(v)
}

fn signed(t: &str) -> Result<i32, String> {
    match t.strip_prefix('-') {
        Some(rest) => {
            let v = i64::from(num(rest)?);
            i32::try_from(-v).map_err(|_| format!("number {t:?} does not fit i32"))
        }
        None => i32::try_from(num(t)?).map_err(|_| format!("number {t:?} does not fit i32")),
    }
}

fn coord(t: &str) -> Result<Coord, String> {
    let Some(body) = t.strip_prefix('@') else {
        return signed(t).map(Coord::Num);
    };
    for (axis, mk) in [("x", Coord::X as fn(i32) -> Coord), ("y", Coord::Y)] {
        if let Some(rest) = body.strip_prefix(axis) {
            let d = match rest.as_bytes().first() {
                None => 0,
                Some(b'+') if !rest[1..].starts_with('-') => signed(&rest[1..])?,
                Some(b'-') if rest.len() > 1 => signed(rest)?,
                _ => return Err(format!("bad position {t:?}")),
            };
            if d == 0 && !rest.is_empty() {
                return Err(format!("{t:?}: write a zero offset as @{axis}"));
            }
            return Ok(mk(d));
        }
    }
    Err(format!("{t:?}: a position is a number, @x±N or @y±N"))
}

fn unit_arg(t: &str) -> Result<UnitArg, String> {
    if let Some((ty, guid)) = t.split_once('/') {
        let ty = ranged(ty, 0, 5, "unit type")? as u8;
        return Ok(UnitArg::Guid {
            ty,
            guid: num(guid)?,
        });
    }
    let Some(body) = t.strip_prefix('@') else {
        return Err(format!(
            "{t:?}: a unit is @player, @<type>[:<class>][#n], @wp[#n] or <type>/<guid>"
        ));
    };
    if body == "player" {
        return Ok(UnitArg::Player);
    }
    let (head, n) = match body.split_once('#') {
        Some((h, n)) => {
            let n = num(n)?;
            if n == 0 {
                return Err(format!("{t:?}: write #0 as nothing"));
            }
            (h, n)
        }
        None => (body, 0),
    };
    if head == "wp" {
        return Ok(UnitArg::Waypoint(n));
    }
    if let Some(row) = head.strip_prefix("su") {
        let row = ranged(row, 0, 0xFFFF, "superuniques row").map_err(|e| {
            format!("{t:?}: {e}; a unit is @player, @<type>[:<class>][#n], @wp[#n], @su<row> or <type>/<guid>")
        })?;
        return Ok(UnitArg::Superunique(row as u16));
    }
    let (ty, class) = match head.split_once(':') {
        Some((ty, c)) => (ty, Some(num(c)?)),
        None => (head, None),
    };
    let ty = ranged(ty, 0, 5, "unit type").map_err(|e| {
        format!("{t:?}: {e}; a unit is @player, @<type>[:<class>][#n], @wp[#n] or <type>/<guid>")
    })? as u8;
    Ok(UnitArg::Nth { ty, class, n })
}

impl fmt::Display for Coord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let off = |f: &mut fmt::Formatter<'_>, axis: &str, d: i32| match d {
            0 => write!(f, "@{axis}"),
            d if d > 0 => write!(f, "@{axis}+{d}"),
            d => write!(f, "@{axis}-{}", d.unsigned_abs()),
        };
        match *self {
            Self::Num(v) => write!(f, "{v}"),
            Self::X(d) => off(f, "x", d),
            Self::Y(d) => off(f, "y", d),
        }
    }
}

impl fmt::Display for UnitArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nth = |f: &mut fmt::Formatter<'_>, n: u32| {
            if n == 0 {
                Ok(())
            } else {
                write!(f, "#{n}")
            }
        };
        match *self {
            Self::Player => write!(f, "@player"),
            Self::Nth { ty, class, n } => {
                write!(f, "@{ty}")?;
                if let Some(c) = class {
                    write!(f, ":{c}")?;
                }
                nth(f, n)
            }
            Self::Waypoint(n) => {
                write!(f, "@wp")?;
                nth(f, n)
            }
            Self::Guid { ty, guid } => write!(f, "{ty}/{guid}"),
            Self::Superunique(row) => write!(f, "@su{row}"),
        }
    }
}

impl fmt::Display for MsgArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Num(v) => write!(f, "{v}"),
            Self::Pos(c) => c.fmt(f),
            Self::Unit(u) => u.fmt(f),
        }
    }
}

/// One `msg` value token: `@x±N` / `@y±N`, a unit reference, or a
/// number (u32). Which values the id takes is the host side's check: message layouts are
/// transport knowledge, not the sim's.
fn msg_arg(t: &str) -> Result<MsgArg, String> {
    if t.starts_with("@x") || t.starts_with("@y") {
        return coord(t).map(MsgArg::Pos);
    }
    if t.starts_with('@') || t.contains('/') {
        return unit_arg(t).map(MsgArg::Unit);
    }
    num(t).map(MsgArg::Num)
}

// ---- directives -----------------------------------------------------------

/// Parses one directive from its tokens (keyword first). Errors name the problem; the
/// caller adds the line.
pub fn parse_directive(toks: &[&str]) -> Result<Directive, String> {
    let Some((&kw, args)) = toks.split_first() else {
        return Err("empty directive".into());
    };
    // Fixed arguments, then the optional `key value...` groups.
    let fixed = |n: usize, usage: &str| -> Result<(&[&str], &[&str]), String> {
        if args.len() < n {
            return Err(format!("`{kw}` needs {n} argument(s): `{kw} {usage}`"));
        }
        Ok(args.split_at(n))
    };
    let exact = |n: usize, usage: &str| -> Result<&[&str], String> {
        if args.len() != n {
            return Err(format!(
                "`{kw}` takes {n} argument(s), got {}: `{kw} {usage}`",
                args.len()
            ));
        }
        Ok(args)
    };
    fn once<T>(slot: &mut Option<T>, v: T, what: &str) -> Result<(), String> {
        if slot.is_some() {
            return Err(format!("`{what}` given twice"));
        }
        *slot = Some(v);
        Ok(())
    }
    let d = match kw {
        "object" => {
            let usage = "<class> <x> <y> [mode <m>]";
            let (a, mut rest) = fixed(3, usage)?;
            let mut mode = None;
            while let [key, r @ ..] = rest {
                match (*key, r) {
                    ("mode", [m, r @ ..]) => {
                        once(&mut mode, ranged(m, 0, 7, "object mode")?, "mode")?;
                        rest = r;
                    }
                    _ => return Err(format!("`object {usage}`: unexpected {key:?}")),
                }
            }
            Directive::Object {
                class: ranged(a[0], 0, 0xFFFF, "objects row")?,
                x: coord(a[1])?,
                y: coord(a[2])?,
                mode,
            }
        }
        "superunique" => {
            let a = exact(3, "<row> <x> <y>")?;
            Directive::Superunique {
                row: ranged(a[0], 0, 0xFFFF, "superuniques row")?,
                x: coord(a[1])?,
                y: coord(a[2])?,
            }
        }
        "missile" => {
            let usage = "<class> <x> <y> <tx> <ty> [skill <id> <level>] [owner <ref>]";
            let (a, mut rest) = fixed(5, usage)?;
            let (mut skill, mut owner) = (None, None);
            while let [key, r @ ..] = rest {
                match (*key, r) {
                    ("skill", [id, lv, r @ ..]) => {
                        let s = (
                            ranged(id, 0, 0xFFFF, "skill id")?,
                            ranged(lv, 0, 255, "skill level")?,
                        );
                        once(&mut skill, s, "skill")?;
                        rest = r;
                    }
                    ("owner", [u, r @ ..]) => {
                        once(&mut owner, unit_arg(u)?, "owner")?;
                        rest = r;
                    }
                    _ => return Err(format!("`missile {usage}`: unexpected {key:?}")),
                }
            }
            Directive::Missile {
                class: ranged(a[0], 0, 0xFFFF, "missiles row")?,
                x: coord(a[1])?,
                y: coord(a[2])?,
                tx: coord(a[3])?,
                ty: coord(a[4])?,
                skill,
                owner,
            }
        }
        "seed-game" => {
            let a = exact(2, "<lo> <hi>")?;
            Directive::SeedGame {
                lo: num(a[0])?,
                hi: num(a[1])?,
            }
        }
        "seed-unit" => {
            let a = exact(3, "<ref> <lo> <hi>")?;
            Directive::SeedUnit {
                unit: unit_arg(a[0])?,
                lo: num(a[1])?,
                hi: num(a[2])?,
            }
        }
        "time" => {
            let a = exact(2, "<period 0..5> <ticks>")?;
            Directive::Time {
                period: ranged(a[0], 0, 5, "period")?,
                ticks: num(a[1])?,
            }
        }
        "pos" => {
            let usage = "<ref> <x> <y> [free]";
            let (a, rest) = fixed(3, usage)?;
            let free = match rest {
                [] => false,
                ["free"] => true,
                _ => return Err(format!("`pos {usage}`")),
            };
            Directive::Pos {
                unit: unit_arg(a[0])?,
                x: coord(a[1])?,
                y: coord(a[2])?,
                free,
            }
        }
        "hop" => {
            let a = exact(3, "<ref> <x> <y>")?;
            Directive::Hop {
                unit: unit_arg(a[0])?,
                x: coord(a[1])?,
                y: coord(a[2])?,
            }
        }
        "warp" => {
            let usage = "<level> [tile <n>]";
            let (a, rest) = fixed(1, usage)?;
            let tile = match rest {
                [] => None,
                ["tile", n] => Some(ranged(n, 0, 0xFF, "tile index")?),
                _ => return Err(format!("`warp {usage}`")),
            };
            Directive::Warp {
                level: ranged(a[0], 0, 0xFFFF, "level id")?,
                tile,
            }
        }
        "item" => {
            let usage = "<code> <x> <y> [quality <q>] [ilvl <n>]";
            let (a, mut rest) = fixed(3, usage)?;
            let code = a[0];
            if !(3..=4).contains(&code.len())
                || !code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            {
                return Err(format!("item code {code:?}: 3-4 of [a-z0-9]"));
            }
            let (mut quality, mut ilvl) = (None, None);
            while let [key, r @ ..] = rest {
                match (*key, r) {
                    ("quality", [q, r @ ..]) => {
                        let i = QUALITIES
                            .iter()
                            .position(|n| n == q)
                            .ok_or_else(|| format!("unknown quality {q:?}"))?;
                        once(&mut quality, i as u8 + 1, "quality")?;
                        rest = r;
                    }
                    ("ilvl", [n, r @ ..]) => {
                        once(&mut ilvl, ranged(n, 1, 99, "ilvl")? as u8, "ilvl")?;
                        rest = r;
                    }
                    _ => return Err(format!("`item {usage}`: unexpected {key:?}")),
                }
            }
            Directive::Item {
                code: code.to_owned(),
                x: coord(a[1])?,
                y: coord(a[2])?,
                quality,
                ilvl,
            }
        }
        "stat" => {
            let a = exact(4, "<ref> <stat> <layer> <i32>")?;
            Directive::Stat {
                unit: unit_arg(a[0])?,
                stat: ranged(a[1], 0, 0xFFFF, "stat id")? as u16,
                layer: ranged(a[2], 0, 0xFFFF, "layer")? as u16,
                value: signed(a[3])?,
            }
        }
        "state" => {
            let a = exact(3, "<ref> <state> on|off")?;
            let on = match a[2] {
                "on" => true,
                "off" => false,
                v => return Err(format!("state {v:?}: on or off")),
            };
            Directive::State {
                unit: unit_arg(a[0])?,
                state: ranged(a[1], 0, 0xFFFF, "state id")? as u16,
                on,
            }
        }
        "freeze" => {
            let a = exact(1, "<seconds>")?;
            Directive::Freeze {
                seconds: ranged(a[0], 0, 3600, "seconds")?,
            }
        }
        "goto" => {
            let usage = "unit [<type>:]<class> | preset <level> [<type>:]<class> [to <level>] [mode <m>] [opening] | [preset <level>] superunique <row>";
            // A trailing `opening`, then `mode <m>` (unit and preset forms).
            let (args, opening) = match args {
                [rest @ .., "opening"] if !rest.is_empty() && rest[0] != "superunique" => {
                    (rest, true)
                }
                _ => (args, false),
            };
            let (args, mode) = match args {
                [rest @ .., "mode", m] if !rest.is_empty() && rest[0] != "superunique" => {
                    (rest, Some(ranged(m, 0, 0xFF, "goto mode")? as u8))
                }
                _ => (args, None),
            };
            let row = match args {
                ["superunique", row] | ["preset", _, "superunique", row] => {
                    Some(ranged(row, 0, 0xFFFF, "superuniques row")? as u16)
                }
                _ => None,
            };
            if let Some(row) = row {
                let preset = match args {
                    ["preset", lv, ..] => Some(ranged(lv, 0, 0xFFFF, "level id")?),
                    _ => None,
                };
                if mode.is_some() || opening {
                    return Err(format!("`goto {usage}`"));
                }
                return Ok(Directive::Goto(GotoTarget {
                    preset,
                    to: None,
                    ty: 1,
                    class: 0,
                    superunique: Some(row),
                    mode: None,
                    opening: false,
                }));
            }
            let (preset, to, id) = match args {
                ["unit", id] => (None, None, *id),
                ["preset", lv, id] => (Some(ranged(lv, 0, 0xFFFF, "level id")?), None, *id),
                ["preset", lv, id, "to", dest] => (
                    Some(ranged(lv, 0, 0xFFFF, "level id")?),
                    Some(ranged(dest, 0, 0xFFFF, "level id")?),
                    *id,
                ),
                _ => return Err(format!("`goto {usage}`")),
            };
            let (ty, class) = match id.split_once(':') {
                Some((t, c)) => {
                    let ty = ranged(t, 1, 5, "goto unit type (1 monster, 2 object, 5 tile)")?;
                    if !matches!(ty, 1 | 2 | 5) {
                        return Err(
                            "goto unit type supports monsters, objects and interaction tiles"
                                .into(),
                        );
                    }
                    (ty, c)
                }
                None => (1, id),
            };
            Directive::Goto(GotoTarget {
                preset,
                to,
                ty: ty as u8,
                class: ranged(class, 0, 0xFFFF, "class")?,
                superunique: None,
                mode,
                opening,
            })
        }
        "msg" => {
            let Some((id, vals)) = args.split_first() else {
                return Err("`msg` needs an id: `msg <id> <value>...`".into());
            };
            let id = ranged(id, 1, 0x70, "msg id")? as u8;
            let args = vals
                .iter()
                .map(|t| msg_arg(t))
                .collect::<Result<Vec<_>, _>>()?;
            Directive::Msg { id, args }
        }
        "operate" => Directive::Operate {
            unit: unit_arg(exact(1, "<ref>")?[0])?,
        },
        "talk" => {
            let Some((npc, choices)) = args.split_first() else {
                return Err("`talk` needs an NPC: `talk <ref> [<choice>...]`".into());
            };
            Directive::Talk {
                npc: unit_arg(npc)?,
                choices: choices
                    .iter()
                    .map(|t| TalkChoice::parse(t))
                    .collect::<Result<_, _>>()?,
            }
        }
        k => {
            return Err(format!(
                "unknown directive {k:?}: one of {}",
                KEYWORDS.join(", ")
            ))
        }
    };
    Ok(d)
}

/// Parses a directive line (`<keyword> <args>...`, tokens separated by
/// spaces and tabs).
pub fn parse_directive_text(line: &str) -> Result<Directive, String> {
    let toks: Vec<&str> = line.split_ascii_whitespace().collect();
    parse_directive(&toks)
}

impl fmt::Display for Directive {
    /// The canonical text: the directive and its arguments as parsed, numbers decimal,
    /// optional arguments in a fixed order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.keyword())?;
        match self {
            Self::Object { class, x, y, mode } => {
                write!(f, " {class} {x} {y}")?;
                if let Some(m) = mode {
                    write!(f, " mode {m}")?;
                }
            }
            Self::Superunique { row, x, y } => write!(f, " {row} {x} {y}")?,
            Self::Missile {
                class,
                x,
                y,
                tx,
                ty,
                skill,
                owner,
            } => {
                write!(f, " {class} {x} {y} {tx} {ty}")?;
                if let Some((s, l)) = skill {
                    write!(f, " skill {s} {l}")?;
                }
                if let Some(o) = owner {
                    write!(f, " owner {o}")?;
                }
            }
            Self::SeedGame { lo, hi } => write!(f, " {lo} {hi}")?,
            Self::SeedUnit { unit, lo, hi } => write!(f, " {unit} {lo} {hi}")?,
            Self::Time { period, ticks } => write!(f, " {period} {ticks}")?,
            Self::Pos { unit, x, y, free } => {
                write!(f, " {unit} {x} {y}")?;
                if *free {
                    write!(f, " free")?;
                }
            }
            Self::Hop { unit, x, y } => write!(f, " {unit} {x} {y}")?,
            Self::Warp { level, tile } => {
                write!(f, " {level}")?;
                if let Some(t) = tile {
                    write!(f, " tile {t}")?;
                }
            }
            Self::Item {
                code,
                x,
                y,
                quality,
                ilvl,
            } => {
                write!(f, " {code} {x} {y}")?;
                if let Some(q) = quality {
                    write!(f, " quality {}", QUALITIES[usize::from(*q) - 1])?;
                }
                if let Some(l) = ilvl {
                    write!(f, " ilvl {l}")?;
                }
            }
            Self::Stat {
                unit,
                stat,
                layer,
                value,
            } => write!(f, " {unit} {stat} {layer} {value}")?,
            Self::State { unit, state, on } => {
                write!(f, " {unit} {state} {}", if *on { "on" } else { "off" })?
            }
            Self::Freeze { seconds } => write!(f, " {seconds}")?,
            Self::Goto(GotoTarget {
                preset,
                superunique: Some(row),
                ..
            }) => match preset {
                Some(l) => write!(f, " preset {l} superunique {row}")?,
                None => write!(f, " superunique {row}")?,
            },
            Self::Goto(t) => match t.preset {
                Some(l) => {
                    write!(f, " preset {l} {}:{}", t.ty, t.class)?;
                    if let Some(d) = t.to {
                        write!(f, " to {d}")?;
                    }
                    if let Some(m) = t.mode {
                        write!(f, " mode {m}")?;
                    }
                    if t.opening {
                        write!(f, " opening")?;
                    }
                }
                None => {
                    write!(f, " unit {}:{}", t.ty, t.class)?;
                    if let Some(m) = t.mode {
                        write!(f, " mode {m}")?;
                    }
                    if t.opening {
                        write!(f, " opening")?;
                    }
                }
            },
            Self::Msg { id, args } => {
                write!(f, " {id}")?;
                for a in args {
                    write!(f, " {a}")?;
                }
            }
            Self::Operate { unit } => write!(f, " {unit}")?,
            Self::Talk { npc, choices } => {
                write!(f, " {npc}")?;
                for c in choices {
                    write!(f, " {c}")?;
                }
            }
        }
        Ok(())
    }
}

/// Parses `spawn`'s arguments (after the keyword): `<class> <x> <y>
/// <kind> [free] [umod <id>...]`.
pub fn parse_spawn(toks: &[&str]) -> Result<Spawn, String> {
    let [class, x, y, kind, rest @ ..] = toks else {
        return Err("`spawn <class> <x> <y> <kind> [free] [umod <id>...]`".into());
    };
    let kind = SpawnKind::ALL
        .into_iter()
        .find(|k| k.name() == *kind)
        .ok_or_else(|| {
            format!("unknown spawn kind {kind:?}: normal, random-boss, champion or unique")
        })?;
    let (free, rest) = match rest {
        ["free", rest @ ..] => (true, rest),
        _ => (false, rest),
    };
    let umods = match rest {
        [] => Vec::new(),
        ["umod", ids @ ..] if !ids.is_empty() => ids
            .iter()
            .map(|t| ranged(t, 1, 255, "umod").map(|v| v as u8))
            .collect::<Result<Vec<u8>, _>>()?,
        _ => return Err("after the kind: `umod <id>...`".into()),
    };
    let ok = match kind {
        SpawnKind::Normal | SpawnKind::RandomBoss => umods.is_empty(),
        SpawnKind::Champion => umods.len() == 1,
        SpawnKind::Unique => (1..=9).contains(&umods.len()),
    };
    if !ok {
        return Err(format!(
            "spawn {}: umods: none for normal and random-boss, one for champion, 1-9 for unique",
            kind.name()
        ));
    }
    Ok(Spawn {
        class: num(class)?,
        x: coord(x)?,
        y: coord(y)?,
        kind,
        free,
        umods,
    })
}

impl fmt::Display for Spawn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "spawn {} {} {} {}",
            self.class,
            self.x,
            self.y,
            self.kind.name()
        )?;
        if self.free {
            write!(f, " free")?;
        }
        if !self.umods.is_empty() {
            write!(f, " umod")?;
            for u in &self.umods {
                write!(f, " {u}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for PokeOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Directive(d) => d.fmt(f),
            Self::Spawn(s) => s.fmt(f),
        }
    }
}

/// Parses `<directive> <args>...` or `spawn ...`.
pub fn parse_op(toks: &[&str]) -> Result<PokeOp, String> {
    match toks.split_first() {
        Some((&"spawn", rest)) => parse_spawn(rest).map(PokeOp::Spawn),
        _ => parse_directive(toks).map(PokeOp::Directive),
    }
}

impl PokeFile {
    /// Parses a `poke 1` file. Blank lines and `#` comments are skipped; ticks must not
    /// decrease.
    pub fn parse(text: &str) -> Result<Self, PokeError> {
        let mut version_seen = false;
        let mut lines: Vec<PokeLine> = Vec::new();
        for (k, raw) in text.lines().enumerate() {
            let line = k + 1;
            let toks: Vec<&str> = raw
                .split_ascii_whitespace()
                .take_while(|t| !t.starts_with('#'))
                .collect();
            if toks.is_empty() {
                continue;
            }
            let err = |message: String| PokeError { line, message };
            if !version_seen {
                if toks != ["poke", "1"] {
                    return Err(err(match toks.as_slice() {
                        ["poke", v] => format!("unknown poke format version {v}"),
                        _ => "the first line must be `poke 1`".into(),
                    }));
                }
                version_seen = true;
                continue;
            }
            let ["at", tick, rest @ ..] = toks.as_slice() else {
                return Err(err("`at <tick> <directive> <args>...`".into()));
            };
            let tick = ranged(tick, 0, MAX_TICK, "tick").map_err(err)?;
            if let Some(prev) = lines.last() {
                if tick < prev.tick {
                    return Err(err(format!(
                        "line at tick {tick} after a line at tick {}",
                        prev.tick
                    )));
                }
            }
            let op = parse_op(rest).map_err(err)?;
            lines.push(PokeLine { tick, op });
        }
        if !version_seen {
            return Err(PokeError {
                line: 0,
                message: "empty poke file: no `poke 1` line".into(),
            });
        }
        Ok(Self { lines })
    }

    /// The canonical text.
    pub fn to_text(&self) -> String {
        let mut s = format!("poke {POKE_FORMAT_VERSION}\n");
        for l in &self.lines {
            s.push_str(&format!("at {} {}\n", l.tick, l.op));
        }
        s
    }
}

// ---- apply ----------------------------------------------------------------

/// A directive's result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PokeResult {
    /// Done; the GUID of the unit it created, if any.
    Ok(Option<u32>),
    /// The game's own function refused (placement, class check, no room).
    Failed,
    /// `failed` with why, e.g. `msg` dropped by the client sender's duplicate filter.
    FailedWith(String),
    /// A reference matched no unit: the reference as written.
    Unresolved(String),
    /// This side cannot run the directive: why.
    Gap(String),
    /// A `goto` step that did not land yet: run the next step after the next tick.
    Pending,
}

impl PokeResult {
    /// `ok`, `failed`, `unresolved` or `gap`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Ok(_) => "ok",
            Self::Failed | Self::FailedWith(_) => "failed",
            Self::Unresolved(_) => "unresolved",
            Self::Gap(_) => "gap",
            Self::Pending => "pending",
        }
    }

    /// The created unit's GUID (only with `ok`).
    pub fn guid(&self) -> Option<u32> {
        match self {
            Self::Ok(g) => *g,
            _ => None,
        }
    }
}

static NO_WAYPOINTS: BTreeSet<u32> = BTreeSet::new();

/// What [`apply`] needs besides the game.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    /// The character (`@player`, `@x`, `@y`, the default missile owner,
    /// the act of `time`, the unit `warp` moves).
    pub player: UnitId,
    /// `objects` rows with operate function 23 (waypoints, for `@wp`); empty: `@wp` never
    /// resolves.
    pub waypoint_classes: &'a BTreeSet<u32>,
    /// The item tables `item` creates from; `None`: `item` is a gap.
    pub items: Option<&'a ItemTables>,
}

impl<'a> Env<'a> {
    /// No waypoint classes, no item tables.
    pub fn new(player: UnitId) -> Self {
        Self {
            player,
            waypoint_classes: &NO_WAYPOINTS,
            items: None,
        }
    }
}

/// The player's path position.
fn player_pos<X: WorldPending>(sim: &WorldSim<X>, env: &Env<'_>) -> (i32, i32) {
    sim.action.sys.hooks.path_position(env.player)
}

fn resolve_coord<X: WorldPending>(
    c: Coord,
    sim: &WorldSim<X>,
    env: &Env<'_>,
) -> Result<i32, String> {
    match c {
        Coord::Num(v) => Ok(v),
        Coord::X(d) => player_pos(sim, env)
            .0
            .checked_add(d)
            .ok_or_else(|| c.to_string()),
        Coord::Y(d) => player_pos(sim, env)
            .1
            .checked_add(d)
            .ok_or_else(|| c.to_string()),
    }
}

/// Resolves a unit argument on the current state (every unit of the unit lists, in
/// ascending GUID order); `Err` holds the reference as written.
pub fn resolve_unit<X: WorldPending>(
    u: UnitArg,
    game: &Game,
    sim: &WorldSim<X>,
    env: &Env<'_>,
) -> Result<UnitId, String> {
    let lists = &game.lists;
    let nth = |ty: UnitType, keep: &dyn Fn(UnitId) -> bool, n: u32| {
        let mut v: Vec<(u32, UnitId)> = lists
            .units_of_type(ty)
            .into_iter()
            .filter(|&id| keep(id))
            .filter_map(|id| lists.unit(id).map(|e| (e.guid, id)))
            .collect();
        v.sort_unstable();
        v.get(n as usize).map(|&(_, id)| id)
    };
    let class = |id: UnitId| sim.action.sys.units.get(id).map(|r| r.class);
    let found = match u {
        UnitArg::Player => lists.unit(env.player).map(|_| env.player),
        UnitArg::Nth { ty, class: c, n } => nth(
            UnitType::ALL[usize::from(ty)],
            &|id| c.is_none_or(|c| class(id) == Some(c)),
            n,
        ),
        UnitArg::Waypoint(n) => nth(
            UnitType::Object,
            &|id| class(id).is_some_and(|c| env.waypoint_classes.contains(&c)),
            n,
        ),
        UnitArg::Guid { ty, guid } => lists.find_unit(UnitType::ALL[usize::from(ty)], guid),
        UnitArg::Superunique(row) => lists
            .units_of_type(UnitType::Monster)
            .into_iter()
            .filter(|&id| {
                sim.world.monsters.get(id).is_some_and(|m| {
                    m.type_flags & crate::monsters::init::type_flag::SUPERUNIQUE != 0
                        && m.boss_hc_idx == row
                })
            })
            .filter_map(|id| lists.unit(id).map(|e| (e.guid, id)))
            .min_by_key(|&(guid, _)| guid)
            .map(|(_, id)| id),
    };
    found.ok_or_else(|| u.to_string())
}

/// Largest move of a `hop` per axis (sub-tiles): a `pos` reaches only the unit's room and
/// its neighbours.
pub const HOP: i32 = 16;

/// The collision bits a landing cell of `pos … free`, `hop` and `goto` must not have: the
/// player's move bits 0x1C09 and the missile-blocking bit 0x4.
pub const LAND_MASK: u16 = crate::path::collision::masks::PLAYER_MOVE | 0x4;

/// The cell of an active room with no `LAND_MASK` bit nearest (tx, ty)
/// (squared distance; row by row from the top-left of the room's
/// sub-tile rectangle, the first found on a tie).
fn nearest_free(a: &crate::drlg::ActiveRoom, tx: i32, ty: i32) -> Option<(i32, i32)> {
    let r = a.subtiles;
    let mut best: Option<(i64, i32, i32)> = None;
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            let free = a.collision.get(x, y).is_some_and(|m| m & LAND_MASK == 0);
            let d2 = i64::from(x - tx).pow(2) + i64::from(y - ty).pow(2);
            if free && best.is_none_or(|(b, _, _)| d2 < b) {
                best = Some((d2, x, y));
            }
        }
    }
    best.map(|(_, x, y)| (x, y))
}

/// [`nearest_free`] in the active room `room`.
fn room_free_cell<X: WorldPending>(
    game: &Game,
    sim: &WorldSim<X>,
    room: RoomId,
    tx: i32,
    ty: i32,
) -> Option<(i32, i32)> {
    let (d, r) = sim.action.sys.hooks.drlg.drlg_room(game, room)?;
    nearest_free(d.room(r).active()?, tx, ty)
}

/// The spots a `hop` from `from` by `step` tries, best first: rings of
/// radius 0, 2, 4, 7 around the full step, then around the half step, then
/// a sidestep of 8 to either side across the step.
pub fn hop_candidates(from: (i32, i32), step: (i32, i32)) -> Vec<(i32, i32)> {
    const DIRS: [(i32, i32); 8] = [
        (1, 1),
        (1, 0),
        (0, 1),
        (-1, 0),
        (0, -1),
        (-1, -1),
        (1, -1),
        (-1, 1),
    ];
    let ring = |(x, y): (i32, i32), out: &mut Vec<(i32, i32)>| {
        out.push((x, y));
        for r in [2, 4, 7] {
            out.extend(DIRS.iter().map(|&(dx, dy)| (x + r * dx, y + r * dy)));
        }
    };
    let (fx, fy) = from;
    let (sx, sy) = step;
    let mut out = Vec::with_capacity(66);
    ring((fx + sx, fy + sy), &mut out);
    ring((fx + sx / 2, fy + sy / 2), &mut out);
    let (px, py) = (sy.signum() * 8, -sx.signum() * 8);
    out.push((fx + px, fy + py));
    out.push((fx - px, fy - py));
    out.retain(|&(x, y)| x >= 0 && y >= 0 && (x, y) != from);
    out
}

/// The room holding (x, y), searched from the room of unit `near` and its neighbours.
fn room_near<X: WorldPending>(
    game: &Game,
    sim: &WorldSim<X>,
    near: UnitId,
    x: i32,
    y: i32,
) -> Option<RoomId> {
    let r = game.lists.unit(near)?.room()?;
    sim.action.sys.hooks.drlg.find_room(game, r, x, y)
}

fn guid_of(game: &Game, u: UnitId) -> Option<u32> {
    game.lists.unit(u).map(|e| e.guid)
}

fn created(game: &Game, u: Option<UnitId>) -> PokeResult {
    match u.and_then(|u| guid_of(game, u)) {
        Some(g) => PokeResult::Ok(Some(g)),
        None => PokeResult::Failed,
    }
}

/// Runs `d` on the game: resolves its references on the current
/// state, then calls the D2RUST path of its row. No I/O; it draws only what
/// that path draws.
pub fn apply<X: WorldPending>(
    game: &mut Game,
    sim: &mut WorldSim<X>,
    env: &Env<'_>,
    d: &Directive,
) -> PokeResult {
    match run(game, sim, env, d) {
        Ok(r) => r,
        Err(reference) => PokeResult::Unresolved(reference),
    }
}

/// [`apply`] for a poke file line (a directive or a `spawn`).
pub fn apply_op<X: WorldPending>(
    game: &mut Game,
    sim: &mut WorldSim<X>,
    env: &Env<'_>,
    op: &PokeOp,
) -> PokeResult {
    match op {
        PokeOp::Directive(d) => apply(game, sim, env, d),
        PokeOp::Spawn(sp) => {
            let pos = resolve_coord(sp.x, sim, env)
                .and_then(|x| resolve_coord(sp.y, sim, env).map(|y| (x, y)));
            let (x, y) = match pos {
                Ok(p) => p,
                Err(r) => return PokeResult::Unresolved(r),
            };
            let act = game
                .lists
                .unit(env.player)
                .and_then(|e| e.room())
                .and_then(|r| game.lists.room(r))
                .map_or(0, |r| r.act);
            let u = spawn_monster(game, sim, act, sp.class, x, y, sp.kind, sp.free, &sp.umods);
            created(game, u)
        }
    }
}

/// Parses `line` (`<directive> <args>...` or `spawn ...`) and applies
/// it: the one-call entry point of the play and dump tools.
pub fn apply_line<X: WorldPending>(
    game: &mut Game,
    sim: &mut WorldSim<X>,
    env: &Env<'_>,
    line: &str,
) -> Result<PokeResult, String> {
    let toks: Vec<&str> = line.split_ascii_whitespace().collect();
    let op = parse_op(&toks)?;
    Ok(apply_op(game, sim, env, &op))
}

/// The `gap` note of `msg` where the runner has no host.
pub const MSG_GAP: &str = "msg needs the host's client queue";

/// The values of a `msg` directive with its references resolved on the current state:
/// positions from the player's path, units as their GUID. `Err`: the reference that
/// matched no unit.
pub fn msg_values<X: WorldPending>(
    game: &Game,
    sim: &WorldSim<X>,
    env: &Env<'_>,
    args: &[MsgArg],
) -> Result<Vec<i64>, String> {
    msg_values_with(args, |a| match a {
        MsgArg::Num(v) => Ok(i64::from(v)),
        MsgArg::Pos(c) => resolve_coord(c, sim, env).map(i64::from),
        MsgArg::Unit(u) => {
            let id = resolve_unit(u, game, sim, env)?;
            guid_of(game, id)
                .map(i64::from)
                .ok_or_else(|| u.to_string())
        }
    })
}

/// [`msg_values`] with the references resolved by `resolve` (a position to its sub-tile, a
/// unit to its GUID; `Err` the reference).
pub fn msg_values_with(
    args: &[MsgArg],
    resolve: impl FnMut(MsgArg) -> Result<i64, String>,
) -> Result<Vec<i64>, String> {
    args.iter().copied().map(resolve).collect()
}

/// The `gap` note of `operate` / `talk` where the runner has no host.
pub const INTERACT_GAP: &str = "operate / talk need the host's dispatcher";

/// The handler calls of `operate` / `talk` with the references resolved on the current
/// state; `None` for another directive. `Err`: the reference that matched no unit.
pub fn interact_calls<X: WorldPending>(
    game: &Game,
    sim: &WorldSim<X>,
    env: &Env<'_>,
    d: &Directive,
) -> Option<Result<(u32, Vec<HandlerCall>), String>> {
    let unit = match d {
        Directive::Operate { unit } => *unit,
        Directive::Talk { npc, .. } => *npc,
        _ => return None,
    };
    let target = |u: UnitArg| -> Result<(u32, u32), String> {
        let id = resolve_unit(u, game, sim, env)?;
        let e = game.lists.unit(id).ok_or_else(|| u.to_string())?;
        Ok((e.ty as u32, e.guid))
    };
    Some((|| {
        let (ty, guid) = target(unit)?;
        let player = guid_of(game, env.player).ok_or_else(|| "@player".to_string())?;
        Ok((guid, interact_calls_with(d, ty, guid, player)))
    })())
}

/// [`interact_calls`] with the target (unit type, GUID) and the player's
/// GUID resolved: the messages of `operate` / `talk`, in order.
pub fn interact_calls_with(d: &Directive, ty: u32, guid: u32, player: u32) -> Vec<HandlerCall> {
    let call = |id: u8, values: &[u32]| HandlerCall {
        id,
        values: values.to_vec(),
    };
    match d {
        Directive::Operate { .. } => vec![call(0x13, &[ty, guid])],
        Directive::Talk { choices, .. } => {
            let mut v = vec![call(0x13, &[ty, guid]), call(0x2F, &[guid])];
            v.extend(choices.iter().map(|c| match *c {
                TalkChoice::Trade => call(0x38, &[1, guid, 0]),
                TalkChoice::Gamble => call(0x38, &[2, guid, 0]),
                TalkChoice::Hire => call(0x38, &[3, guid, player]),
                TalkChoice::Action(n) => call(0x38, &[n, guid, 0]),
                TalkChoice::Quest(m) => call(0x31, &[guid, u32::from(m)]),
                TalkChoice::Close => call(0x30, &[guid]),
            }));
            v
        }
        _ => Vec::new(),
    }
}

fn run<X: WorldPending>(
    game: &mut Game,
    sim: &mut WorldSim<X>,
    env: &Env<'_>,
    d: &Directive,
) -> Result<PokeResult, String> {
    let c = |v: Coord, sim: &WorldSim<X>| resolve_coord(v, sim, env);
    Ok(match d {
        Directive::Object { class, x, y, mode } => {
            let (x, y) = (c(*x, sim)?, c(*y, sim)?);
            let Some(room) = room_near(game, sim, env.player, x, y) else {
                return Ok(PokeResult::Failed);
            };
            // Allocates an object unit (added to its lists), mode as given (default 0),
            // with the monster state lent because an object's init function may spawn
            // monsters. This is the creation path the game's own objects go through
            // (`View::create_object`, as the population and the quests use), so the
            // per-kind init (control record, init function) runs on the allocation's
            // room and (x, y) before the unit is added. A game without object state, a
            // mode beyond a byte or a class past the objects rows gets the bare
            // allocation (the init dispatch's own checks decide).
            let class = *class;
            let mode = mode.unwrap_or(0);
            let u = sim.lend(|a| {
                a.with(game, |g, v| match u8::try_from(mode) {
                    Ok(m)
                        if v.h.objects.is_some()
                            && class <= u32::from(crate::world::objects::CLASS_BOUND) =>
                    {
                        v.create_object(g, room, class, x, y, m)
                    }
                    _ => {
                        let req = crate::units::lifecycle::AllocRequest {
                            ty: UnitType::Object,
                            class,
                            room: Some(room),
                            add: true,
                            fixed_guid: None,
                            mode,
                            allied: false,
                        };
                        v.allocate(g, &req, x, y)
                    }
                })
            });
            created(game, u)
        }
        Directive::Superunique { row, x, y } => {
            let (x, y) = (c(*x, sim)?, c(*y, sim)?);
            let Some(room) = room_near(game, sim, env.player, x, y) else {
                return Ok(PokeResult::Failed);
            };
            let su = *row as i32;
            let u = sim.population(game, |cx| preset::superunique(cx, room, x, y, su));
            created(game, u)
        }
        Directive::Missile {
            class,
            x,
            y,
            tx,
            ty,
            skill,
            owner,
        } => {
            let (x, y, tx, ty) = (c(*x, sim)?, c(*y, sim)?, c(*tx, sim)?, c(*ty, sim)?);
            let owner = match owner {
                Some(o) => resolve_unit(*o, game, sim, env)?,
                None => env.player,
            };
            let (skill, level) = skill.unwrap_or((0, 0));
            let p = MissileParams {
                flags: pf::POSITION | pf::TARGET_ABSOLUTE,
                owner: Some(owner),
                origin: Some(owner),
                class: *class as i32,
                x,
                y,
                target_x: tx,
                target_y: ty,
                skill: skill as i32,
                level: level as i32,
                ..MissileParams::default()
            };
            let u = sim
                .lend(|a| a.missiles(game, |g, cx| create_missile(g, cx, &p)))
                .flatten();
            created(game, u)
        }
        Directive::SeedGame { lo, hi } => {
            sim.action.sys.hooks.game_seed.set(*lo, *hi);
            PokeResult::Ok(None)
        }
        Directive::SeedUnit { unit, lo, hi } => {
            let u = resolve_unit(*unit, game, sim, env)?;
            match sim.action.sys.units.get_mut(u) {
                Some(r) => {
                    r.seed.set(*lo, *hi);
                    PokeResult::Ok(None)
                }
                None => PokeResult::Failed,
            }
        }
        Directive::Time { period, ticks } => {
            let act = game
                .lists
                .unit(env.player)
                .and_then(|e| e.room())
                .and_then(|r| game.lists.room(r))
                .map(|r| r.act);
            match act.and_then(|a| game.lists.act_mut(a)) {
                Some(a) => {
                    a.environment.period = *period;
                    a.environment.ticks = *ticks;
                    PokeResult::Ok(None)
                }
                None => PokeResult::Failed,
            }
        }
        Directive::Pos { unit, x, y, free } => {
            let u = resolve_unit(*unit, game, sim, env)?;
            let (mut x, mut y) = (c(*x, sim)?, c(*y, sim)?);
            if !sim.action.sys.hooks.path_has(u) {
                return Ok(PokeResult::Failed);
            }
            let Some(room) = room_near(game, sim, u, x, y) else {
                return Ok(PokeResult::Failed);
            };
            if *free {
                // The nearest free, missile-passable cell of that room.
                match room_free_cell(game, sim, room, x, y) {
                    Some(cell) => (x, y) = cell,
                    None => return Ok(PokeResult::Failed),
                }
            }
            // A `pos` sends no S->C 0x15 message, so there is no reassign mark here (nor
            // in `hop`, which is a `pos` per candidate).
            sim.lend(|a| a.with(game, |g, v| PathCtx::of(v, g).teleport(u, Some(room), x, y)));
            PokeResult::Ok(None)
        }
        Directive::Hop { unit, x, y } => {
            let u = resolve_unit(*unit, game, sim, env)?;
            let (tx, ty) = (c(*x, sim)?, c(*y, sim)?);
            if !sim.action.sys.hooks.path_has(u) {
                return Ok(PokeResult::Failed);
            }
            let from = sim.action.sys.hooks.path_position(u);
            let (dx, dy) = (tx - from.0, ty - from.1);
            if dx.abs() <= 1 && dy.abs() <= 1 {
                return Ok(PokeResult::Ok(None));
            }
            let step = (dx.clamp(-HOP, HOP), dy.clamp(-HOP, HOP));
            for (cx, cy) in hop_candidates(from, step) {
                let Some(room) = room_near(game, sim, u, cx, cy) else {
                    continue;
                };
                // Only a free, missile-passable cell (`LAND_MASK`).
                if sim
                    .action
                    .sys
                    .hooks
                    .drlg
                    .collision(game, room, cx, cy)
                    .is_none_or(|m| m & LAND_MASK != 0)
                {
                    continue;
                }
                let errors = sim.action.sys.hooks.errors.len();
                sim.lend(|a| {
                    a.with(game, |g, v| {
                        PathCtx::of(v, g).teleport(u, Some(room), cx, cy)
                    })
                });
                // No reassign mark: a `hop` is a `pos` per candidate,
                // which sends no S->C 0x15, so the client never runs the
                // code-7 neutral request and a player hopped into town
                // keeps mode 1 (not TN).
                if sim.action.sys.hooks.path_position(u) != from {
                    return Ok(PokeResult::Ok(None));
                }
                // a refused spot (blocked): try the next; the refusal is the probe's
                sim.action.sys.hooks.errors.truncate(errors);
            }
            PokeResult::Failed
        }
        Directive::Warp { level, tile } => {
            let player = env.player;
            let tile = tile.unwrap_or(0);
            // The same-act warp, else the act change.
            let r = sim.lend(|a| {
                a.with(game, |g, v| {
                    level_warp(PathCtx::of(v, g), player, *level, tile)
                        .unwrap_or_else(|| act_change::run(PathCtx::of(v, g), player, *level, tile))
                })
            });
            if r {
                PokeResult::Ok(None)
            } else {
                PokeResult::Failed
            }
        }
        Directive::Item {
            code,
            x,
            y,
            quality,
            ilvl,
        } => {
            let Some(tables) = env.items else {
                return Ok(PokeResult::Gap(
                    "item: no item tables in this runner".into(),
                ));
            };
            let (x, y) = (c(*x, sim)?, c(*y, sim)?);
            let mut key = [b' '; 4];
            key[..code.len()].copy_from_slice(code.as_bytes());
            let Some(index) = tables.find_code(key) else {
                return Ok(PokeResult::Failed);
            };
            let Some(room) = room_near(game, sim, env.player, x, y) else {
                return Ok(PokeResult::Failed);
            };
            let u = sim.lend(|a| {
                a.with(game, |g, v| {
                    create_ground_item(g, v, tables, room, index, *quality, *ilvl, x, y, env.player)
                })
            });
            created(game, u)
        }
        Directive::Stat {
            unit,
            stat,
            layer,
            value,
        } => {
            let u = resolve_unit(*unit, game, sim, env)?;
            let sys = &mut sim.action.sys;
            if sys.stats.unit_list(u).is_none() {
                return Ok(PokeResult::Failed);
            }
            sys.stats.unit_set(&mut sys.hooks, u, *stat, *value, *layer);
            PokeResult::Ok(None)
        }
        Directive::State { unit, state, on } => {
            let u = resolve_unit(*unit, game, sim, env)?;
            let sys = &sim.action.sys;
            if usize::from(*state) >= sys.stats.data().states.count()
                || sys.stats.unit_list(u).is_none()
            {
                return Ok(PokeResult::Failed);
            }
            // The state toggle, then the insert into the update queue.
            let queued = sim.action.with(game, |g, v| {
                v.set_state(u, *state, *on);
                g.lists.queue_update(u).is_ok()
            });
            if queued {
                PokeResult::Ok(None)
            } else {
                PokeResult::Failed
            }
        }
        Directive::Freeze { .. } => PokeResult::Ok(None),
        Directive::Goto(t) => goto_step(game, sim, env, t, &mut GotoWalk::default()),
        // The message goes through the host's client sender; a runner without one cannot
        // run it.
        Directive::Msg { .. } => PokeResult::Gap(MSG_GAP.into()),
        // The handlers are the host's dispatcher.
        Directive::Operate { .. } | Directive::Talk { .. } => PokeResult::Gap(INTERACT_GAP.into()),
    })
}

/// The level id of the room holding unit `u`.
fn unit_level<X: WorldPending>(game: &Game, sim: &WorldSim<X>, u: UnitId) -> Option<u32> {
    let room = game.lists.unit(u)?.room()?;
    sim.action.sys.hooks.drlg.level_id(game, room)
}

/// The destination level of the warp link of the room holding `u` whose
/// lvlwarp `Id` is the unit's class (a type-5 interaction tile).
fn warp_target_level<X: WorldPending>(
    game: &mut Game,
    sim: &mut WorldSim<X>,
    u: UnitId,
) -> Option<u32> {
    let room = game.lists.unit(u)?.room()?;
    let act = game.lists.room(room)?.act;
    let class = sim.action.sys.units.get(u)?.class;
    sim.action
        .sys
        .hooks
        .drlg
        .with_act(act, &mut game.lists, |d, svc| {
            let r = d.drlg_room_of(room)?;
            let link = d.room(r).warp_links.iter().find(|l| {
                svc.data
                    .warps
                    .get(l.lvlwarp_row)
                    .is_some_and(|w| w.id as u32 == class)
            })?;
            Some(d.level(d.room(link.target).level).id)
        })
        .flatten()
}

/// Populates a room the walk reaches: its presets (the warp tiles are
/// created by this first walk, not by streaming alone), objects and monsters.
fn populate_presets<X: WorldPending>(game: &mut Game, sim: &mut WorldSim<X>, room: RoomId) {
    // The room's whole off-tick population (as a portal arrival runs it):
    // the preset walk is taken once per room, and taking it must also place
    // the preset monsters (superuniques, act bosses) and mark the room
    // populated. Spawning only its objects and tiles here left the room's
    // bosses unplaced for good.
    sim.lend(|a| a.with(game, |g, v| v.populate_off_tick(g, room)));
}

/// After a placement: when the player's cell has a `LAND_MASK` bit, place again, exact, on
/// the nearest cell of its room without one.
fn settle_landing<X: WorldPending>(game: &mut Game, sim: &mut WorldSim<X>, player: UnitId) {
    let Some(room) = game.lists.unit(player).and_then(|e| e.room()) else {
        return;
    };
    let (x, y) = sim.action.sys.hooks.path_position(player);
    if sim
        .action
        .sys
        .hooks
        .drlg
        .collision(game, room, x, y)
        .is_none_or(|m| m & LAND_MASK == 0)
    {
        return;
    }
    if let Some((nx, ny)) = room_free_cell(game, sim, room, x, y) {
        sim.lend(|a| {
            a.with(game, |g, v| {
                place_unit(PathCtx::of(v, g), player, Some(room), nx, ny, true, false)
            })
        });
    }
}

/// Streams and populates the rooms near `room` (its DRLG near list): a
/// door stands on a room edge, and the room behind it is not built until
/// the player walks up to it.
fn stream_near<X: WorldPending>(game: &mut Game, sim: &mut WorldSim<X>, room: RoomId) {
    let Some(act) = game.lists.room(room).map(|r| r.act) else {
        return;
    };
    let rooms = sim
        .action
        .sys
        .hooks
        .drlg
        .with_act(act, &mut game.lists, |d, svc| {
            let cur = d.drlg_room_of(room)?;
            let near = d.room(cur).near().unwrap_or(&[]).to_vec();
            Some(
                near.into_iter()
                    .filter_map(|r| d.stream_room(svc, r).ok().flatten())
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .unwrap_or_default();
    for r in rooms {
        populate_presets(game, sim, r);
    }
}

/// Sub-tiles from a door's opening a `goto … opening` lands the player,
/// across the door on its positive side: close enough that a click on the
/// door opens it without a walk.
pub const OPENING_SIDE: i32 = 2;

/// The landing point of `goto … opening` for the door `door`: the centre of
/// its opening, [`OPENING_SIDE`] sub-tiles out across the door (+x for a
/// door that runs along y, +y for one that runs along x), or up to two
/// further out when that cell is taken ([`LAND_MASK`]). The opening is
/// the longest run of the door's footprint cells along its long side with
/// no wall bit (0x1) under the door: a long door can stand over wall cells,
/// so its position need not be the middle of the way through (the Maggot
/// Lair's slime door 230 is 7 long over a 3-wide tunnel, its position two
/// cells from the tunnel's middle). `None`: not an object with a footprint,
/// no opening, or no free landing cell (the plain `goto` placement then).
fn door_opening<X: WorldPending>(
    game: &Game,
    sim: &WorldSim<X>,
    door: UnitId,
) -> Option<(i32, i32)> {
    let class = sim.action.sys.units.get(door)?.class;
    let shape = sim
        .action
        .sys
        .hooks
        .object_footprint_shape(Some(UnitType::Object), class)?;
    let (x, y) = sim.action.sys.hooks.path_position(door);
    let along_x = shape.size_x > shape.size_y;
    let long = shape.size_x.max(shape.size_y) as i32;
    let cell = |i: i32| if along_x { (x + i, y) } else { (x, y + i) };
    let open = |i: i32| {
        let (cx, cy) = cell(i);
        room_near(game, sim, door, cx, cy).is_some_and(|r| {
            sim.action
                .sys
                .hooks
                .drlg
                .collision(game, r, cx, cy)
                .is_some_and(|m| m & 0x1 == 0)
        })
    };
    // The longest run of open cells, footprint left / top first.
    let mut best: Option<(i32, i32)> = None;
    let mut run: Option<(i32, i32)> = None;
    for i in -(long / 2)..=(long - 1 - long / 2) {
        run = if open(i) {
            Some(run.map_or((i, i), |(a, _)| (a, i)))
        } else {
            None
        };
        if let Some((a, b)) = run {
            if best.is_none_or(|(c, d)| b - a > d - c) {
                best = Some((a, b));
            }
        }
    }
    let (a, b) = best?;
    let (cx, cy) = cell((a + b) / 2);
    let out = |k: i32| if along_x { (cx, cy + k) } else { (cx + k, cy) };
    // The first landing cell on the positive side the player may stand on
    // (a monster or a wall right behind the door pushes it out a little).
    (OPENING_SIDE..=OPENING_SIDE + 2)
        .map(out)
        .find(|&(lx, ly)| {
            room_near(game, sim, door, lx, ly).is_some_and(|r| {
                sim.action
                    .sys
                    .hooks
                    .drlg
                    .collision(game, r, lx, ly)
                    .is_some_and(|m| m & LAND_MASK == 0)
            })
        })
}

/// One step of a `goto` walk: `Pending` until the target is found and the player placed
/// next to it (`Ok` with the target's GUID) or the walk ends `Failed`. `walk` is the state
/// the runner keeps between steps (a fresh one for a new `goto`).
pub fn goto_step<X: WorldPending>(
    game: &mut Game,
    sim: &mut WorldSim<X>,
    env: &Env<'_>,
    t: &GotoTarget,
    walk: &mut GotoWalk,
) -> PokeResult {
    let player = env.player;
    walk.steps += 1;
    if walk.steps > GOTO_MAX_STEPS {
        return PokeResult::Failed;
    }
    let Some(here) = unit_level(game, sim, player) else {
        return PokeResult::Failed;
    };
    let goal = *walk.goal.get_or_insert(t.preset.unwrap_or(here));
    // The warp to a preset's level, at the first step only.
    if walk.steps == 1 && here != goal {
        let warp = Directive::Warp {
            level: goal,
            tile: None,
        };
        return match apply(game, sim, env, &warp) {
            PokeResult::Ok(_) => PokeResult::Pending,
            _ => PokeResult::Failed,
        };
    }
    // The room's warp tiles are created by its first preset walk, not by
    // streaming alone: run it for the player's room before looking for the
    // target.
    if let Some(room) = game.lists.unit(player).and_then(|e| e.room()) {
        if let Some(act) = game.lists.room(room).map(|r| r.act) {
            // Every active room, not just the player's: a neighbour marked seen
            // below is never walked into, so its tiles must exist already.
            for r in game.lists.active_rooms(act) {
                populate_presets(game, sim, r);
            }
        }
    }
    // The first unit of the target's type and class in the goal level, ascending GUID.
    let ty = UnitType::ALL[usize::from(t.ty)];
    let mut found: Vec<(u32, UnitId)> = game
        .lists
        .units_of_type(ty)
        .into_iter()
        .filter(|&u| {
            if let Some(row) = t.superunique {
                // The world state is lent to the hooks only inside a
                // `lend`: read the monster data from the world itself.
                return sim.world.monsters.get(u).is_some_and(|m| {
                    m.type_flags & crate::monsters::init::type_flag::SUPERUNIQUE != 0
                        && m.boss_hc_idx == row
                });
            }
            t.to.is_some()
                || sim
                    .action
                    .sys
                    .units
                    .get(u)
                    .is_some_and(|r| r.class == t.class)
        })
        .filter(|&u| {
            t.mode.is_none_or(|m| {
                sim.action
                    .sys
                    .units
                    .get(u)
                    .is_some_and(|r| r.mode == u32::from(m))
            })
        })
        .filter(|&u| unit_level(game, sim, u) == Some(goal))
        .filter_map(|u| guid_of(game, u).map(|g| (g, u)))
        .collect();
    // A warp tile's class is its lvlwarp `Id`, which several level links may
    // share: `to <level>` keeps only a tile whose room's warp link leads to
    // that level (the generated link cases need one exact exit). A level may
    // list several directional variants of one cave entrance in Levels.txt
    // and use only one of them in its DS1: keep the requested class when the
    // level has it, else take another class that reaches the destination.
    if let Some(dest) = t.to {
        found.retain(|&(_, u)| warp_target_level(game, sim, u) == Some(dest));
        let exact: Vec<(u32, UnitId)> = found
            .iter()
            .copied()
            .filter(|&(_, u)| {
                sim.action
                    .sys
                    .units
                    .get(u)
                    .is_some_and(|r| r.class == t.class)
            })
            .collect();
        if !exact.is_empty() {
            found = exact;
        }
    }
    found.sort_unstable();
    if let Some(&(guid, target)) = found.first() {
        let Some(room) = game.lists.unit(target).and_then(|e| e.room()) else {
            return PokeResult::Failed;
        };
        let (mut x, mut y) = sim.action.sys.hooks.path_position(target);
        if t.opening {
            stream_near(game, sim, room);
            if let Some(p) = door_opening(game, sim, target) {
                (x, y) = p;
            }
        }
        let placed = sim.lend(|a| {
            a.with(game, |g, v| {
                place_unit(PathCtx::of(v, g), player, Some(room), x, y, false, false)
            })
        });
        // Settle on a missile-passable cell.
        if placed {
            settle_landing(game, sim, player);
        }
        return if placed {
            PokeResult::Ok(Some(guid))
        } else {
            PokeResult::Failed
        };
    }
    // Mark what is seen, then one hop towards the nearest room of the goal level not
    // seen.
    let Some(room) = game.lists.unit(player).and_then(|e| e.room()) else {
        return PokeResult::Failed;
    };
    let Some(act) = game.lists.room(room).map(|r| r.act) else {
        return PokeResult::Failed;
    };
    let hop = {
        let Some((d, cur)) = sim.action.sys.hooks.drlg.drlg_room(game, room) else {
            return PokeResult::Failed;
        };
        let key = |r: DrlgRoomId| {
            let dr = d.room(r);
            (d.level(dr.level).id, dr.rect.x, dr.rect.y)
        };
        let near = |r: DrlgRoomId| d.room(r).near().unwrap_or(&[]).to_vec();
        walk.seen.insert(key(cur));
        for n in near(cur) {
            if d.room(n).active().is_some() {
                walk.seen.insert(key(n));
            }
        }
        // Breadth first over the near arrays, in their stored order.
        let mut parent: BTreeMap<DrlgRoomId, DrlgRoomId> = BTreeMap::new();
        let mut queue = VecDeque::from([cur]);
        let mut done = BTreeSet::from([cur]);
        let mut goal_room = None;
        while let Some(r) = queue.pop_front() {
            let (lv, _, _) = key(r);
            if lv == goal && !walk.seen.contains(&key(r)) {
                goal_room = Some(r);
                break;
            }
            for n in near(r) {
                if !walk.blocked.contains(&key(n)) && done.insert(n) {
                    parent.insert(n, r);
                    queue.push_back(n);
                }
            }
        }
        let Some(mut h) = goal_room else {
            // The rooms-near search found no unseen room of the goal level.
            // Some level shapes have no near link to the room the target
            // stands in (the Arcane Sanctuary's platforms, a boss room
            // behind a one-way exit): stream and populate the whole level
            // once, then look again.
            if walk.swept {
                // Every room of the goal level was loaded and searched (a
                // room the player could not stand in was still loaded): the
                // level holds no such unit (a door table row whose wall tile
                // this map does not use, or a door its room roll left out).
                if !walk.unloaded.iter().any(|&(lv, _, _)| lv == goal) {
                    return PokeResult::FailedWith(format!("{GOTO_NONE_IN_LEVEL} {goal}"));
                }
                return PokeResult::Failed;
            }
            walk.swept = true;
            let streamed = sim
                .action
                .sys
                .hooks
                .drlg
                .with_act(act, &mut game.lists, |d, svc| {
                    let cur = d.drlg_room_of(room)?;
                    let mut out = Vec::new();
                    let mut unloaded = Vec::new();
                    for r in d.level_rooms(d.room(cur).level) {
                        if let Ok(Some(id)) = d.stream_room(svc, r) {
                            out.push(id);
                        } else {
                            let dr = d.room(r);
                            unloaded.push((d.level(dr.level).id, dr.rect.x, dr.rect.y));
                        }
                    }
                    Some((out, unloaded))
                })
                .flatten();
            let (rooms, unloaded) = streamed.unwrap_or_default();
            walk.unloaded.extend(unloaded);
            for r in rooms {
                populate_presets(game, sim, r);
            }
            return PokeResult::Pending;
        };
        while let Some(&p) = parent.get(&h) {
            if p == cur {
                break;
            }
            h = p;
        }
        (key(h), h)
    };
    let (hkey, h) = hop;
    // The hop's room is streamed first: a level is generated as the player
    // walks into it, so a target room that is not active yet (a room of the
    // same level beyond the loaded neighbours) must be built before it can
    // be landed in.
    let streamed = sim
        .action
        .sys
        .hooks
        .drlg
        .with_act(act, &mut game.lists, |d, svc| {
            d.stream_room(svc, h).ok().flatten()?;
            d.room(h).active().map(|a| a.id)
        });
    let Some(Some(hroom)) = streamed else {
        walk.blocked.insert(hkey);
        walk.seen.insert(hkey);
        walk.unloaded.insert(hkey);
        return PokeResult::Pending;
    };
    // The hop room's first preset walk creates its warp tiles before the
    // next step's target search.
    populate_presets(game, sim, hroom);
    // The free, missile-passable cell of H nearest its centre.
    let cell = sim
        .action
        .sys
        .hooks
        .drlg
        .drlg_room(game, hroom)
        .and_then(|(d, r)| {
            let a = d.room(r).active()?;
            let rect = a.subtiles;
            let cell = nearest_free(a, rect.x + rect.w / 2, rect.y + rect.h / 2);
            Some(cell)
        })
        .flatten();
    let Some((x, y)) = cell else {
        walk.seen.insert(hkey);
        walk.blocked.insert(hkey);
        return PokeResult::Pending;
    };
    let placed = sim.lend(|a| {
        a.with(game, |g, v| {
            place_unit(PathCtx::of(v, g), player, Some(hroom), x, y, false, false)
        })
    });
    // A refused hop, or one that left the player outside H, marks H seen.
    let landed = placed
        && game
            .lists
            .unit(player)
            .and_then(|e| e.room())
            .is_some_and(|r| r == hroom);
    if !landed {
        walk.blocked.insert(hkey);
        walk.seen.insert(hkey);
    }
    PokeResult::Pending
}

/// The name on a poked ear.
const EAR_VICTIM: &[u8] = b"Poked ear";

/// Creates an item with spawn mode 3 (ground) and init flags 1 in `room`, then runs the
/// path part of adding a unit at (x, y) (the placement the drop path uses,
/// `wiring::economy::death`).
#[allow(clippy::too_many_arguments)]
fn create_ground_item<X: crate::wiring::action::Pending>(
    game: &mut Game,
    v: &mut crate::wiring::action::View<'_, X>,
    tables: &ItemTables,
    room: RoomId,
    index: usize,
    quality: Option<u8>,
    ilvl: Option<u8>,
    x: i32,
    y: i32,
    player: UnitId,
) -> Option<UnitId> {
    let h = &mut *v.h;
    let mut fields = GameFields::from_action(
        h.game_seed,
        &h.ai_info,
        v.data.expansion,
        std::mem::take(&mut h.uniques),
    );
    let mut items = std::mem::take(&mut h.items);
    let made = {
        let mut econ = Economy {
            game: &mut *game,
            units: &mut *v.units,
            stats: &mut *v.stats,
            data: v.data,
            hooks: &mut *v.h,
            fields: &mut fields,
            tables,
            items: &mut items,
        };
        let mut rq = ItemRequest {
            ilvl: ilvl.map_or(1, i32::from),
            item: index as i32,
            quality: quality.unwrap_or(0),
            format: ItemGame::item_format(&*econ.fields),
            ..ItemRequest::default()
        };
        // An ear is made from a slain player; with no fight, the poking
        // player stands in as the victim.
        if tables.is_type(index, crate::items::ty::PLAY as i16) {
            let mut name = [0u8; 16];
            name[..EAR_VICTIM.len()].copy_from_slice(EAR_VICTIM);
            rq.unit = econ.request_unit(player, Some((name, Some(false))));
        }
        econ.create_item(
            &mut rq,
            false,
            ItemSpawn {
                room: Some(room),
                mode: 3,
                init_flags: 1,
            },
        )
        .ok()
    };
    let h = &mut *v.h;
    h.items = items;
    h.game_seed = fields.seed;
    h.uniques = std::mem::take(&mut fields.uniques);
    let u = made?;
    v.path_place(game, u, x, y);
    Some(u)
}

/// Spawns a monster of `class` and the given kind at (x, y) in the active room of `act`
/// that holds the point, through the population creation calls. Returns the spawned unit
/// (for the boss kinds, the boss itself rather than its minions), or `None` when no room
/// holds the point or a call fails.
#[allow(clippy::too_many_arguments)]
pub fn spawn_monster<X: WorldPending>(
    game: &mut Game,
    ev: &mut WorldSim<X>,
    act: u8,
    class: u32,
    x: i32,
    y: i32,
    kind: SpawnKind,
    free: bool,
    umods: &[u8],
) -> Option<UnitId> {
    let room = game.lists.active_rooms(act).into_iter().find(|&r| {
        ev.action
            .sys
            .hooks
            .drlg
            .subtiles(game, r)
            .is_some_and(|s| x >= s.x && x < s.x + s.w && y >= s.y && y < s.y + s.h)
    })?;
    let class = i32::try_from(class).ok()?;
    let unit = match kind {
        SpawnKind::Normal => ev.population(game, |cx| {
            // `free`: the population's own retry radius (rings 3, 6, 9, 12)
            // when the exact point is blocked.
            let r = if free { 4 } else { -1 };
            placement::place_at(cx, room, None, x, y, class, 1, r, 0).unit()
        })?,
        SpawnKind::RandomBoss => ev.population(game, |cx| {
            let b = pop_spawn::random_boss(cx, room, None, class, true, x, y, false)?;
            pop_spawn::champion_minions(cx, None, b, class);
            Some(b)
        })?,
        SpawnKind::Champion => {
            let b = ev.population(game, |cx| {
                pop_spawn::boss_spawn(cx, room, None, x, y, None, class, false)
            })?;
            let umod = *umods.first()?;
            ev.init(game, |cx, h| init::champion_pack_member(cx, h, b, umod));
            ev.population(game, |cx| pop_spawn::champion_minions(cx, None, b, class));
            b
        }
        SpawnKind::Unique => {
            let b = ev.population(game, |cx| {
                pop_spawn::boss_spawn(cx, room, None, x, y, None, class, false)
            })?;
            ev.init(game, |_, h| {
                for &u in umods {
                    h.monsters().entry(b).push_umod(u);
                }
            });
            ev.population(game, |cx| {
                pop_spawn::boss_minions_and_init(cx, b, 3, 6, None)
            });
            b
        }
    };
    Some(unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One line of every directive, with every optional argument and
    /// every reference form.
    const ALL: &[&str] = &[
        "goto unit 1:156",
        "goto preset 107 2:376",
        "goto preset 2 5:4",
        "goto preset 2 5:4 to 8",
        "goto preset 28 2:15 mode 0",
        "goto preset 62 2:230 mode 0 opening",
        "goto unit 2:230 opening",
        "goto unit 2:15 mode 2",
        "goto superunique 40",
        "goto preset 8 superunique 40",
        "object 119 100 200",
        "object 119 @x+5 @y-3 mode 1",
        "superunique 3 @x @y+10",
        "missile 7 @x @y @x+10 @y",
        "missile 7 100 200 110 200 skill 36 1 owner @1:19#2",
        "missile 7 100 200 110 200 owner 1/42",
        "seed-game 305419896 666",
        "seed-unit @1:19 1 2",
        "seed-unit @wp#1 1 2",
        "time 5 1024",
        "pos @player @x+3 @y",
        "pos @player 100 200 free",
        "hop @player 5100 @y-40",
        "warp 3",
        "warp 3 tile 2",
        "item hp1 @x @y",
        "item rin @x @y quality rare ilvl 85",
        "stat @player 0 0 -5",
        "stat @0 14 0 500000",
        "state @player 1 on",
        "state 1/9 2 off",
        "freeze 3",
        "msg 1 @x+2 @y",
        "msg 6 1 @1",
        "msg 60 36 1 4294967295",
        "msg 96",
        "operate @wp",
        "operate @2:267",
        "talk @1:148",
        "talk 1/12 trade close",
        "talk @1:148 gamble hire action:7 quest:92 close",
    ];

    #[test]
    fn a_free_spawn_round_trips() {
        let op = parse_op(&["spawn", "19", "1", "2", "normal", "free"]).unwrap();
        assert_eq!(op.to_string(), "spawn 19 1 2 normal free");
        assert!(matches!(op, PokeOp::Spawn(s) if s.free));
        // `free` still takes normal's empty umod list.
        assert!(parse_op(&["spawn", "19", "1", "2", "normal", "free", "umod", "1"]).is_err());
    }

    #[test]
    fn every_directive_round_trips() {
        let mut seen = BTreeSet::new();
        for line in ALL {
            let d = parse_directive_text(line).unwrap_or_else(|e| panic!("{line}: {e}"));
            seen.insert(d.keyword());
            let text = d.to_string();
            assert_eq!(&text, line, "canonical");
            assert_eq!(parse_directive_text(&text).unwrap(), d);
        }
        assert_eq!(seen.len(), KEYWORDS.len());
        // Hex and reordered optional arguments come back canonical.
        let d =
            parse_directive_text("missile 0x7 0x64 200 110 200 owner @player skill 36 1").unwrap();
        assert_eq!(
            d.to_string(),
            "missile 7 100 200 110 200 skill 36 1 owner @player"
        );
        assert_eq!(
            parse_directive_text("item rin 1 2 ilvl 5 quality magic")
                .unwrap()
                .to_string(),
            "item rin 1 2 quality magic ilvl 5"
        );
        // `goto`: the type defaults to 1 and is written out.
        assert_eq!(
            parse_directive_text("goto unit 5").unwrap().to_string(),
            "goto unit 1:5"
        );
        assert_eq!(
            parse_directive_text("goto preset 107 376")
                .unwrap()
                .to_string(),
            "goto preset 107 1:376"
        );
    }

    #[test]
    fn hop_tries_the_full_step_first_then_the_half_then_a_sidestep() {
        let c = hop_candidates((100, 100), (16, -16));
        assert_eq!(c[0], (116, 84), "the full step itself first");
        assert_eq!(c[1], (118, 86), "then its ring, radius 2 first");
        assert_eq!(c.len(), 25 + 25 + 2);
        assert_eq!(c[25], (108, 92), "then the half step");
        assert_eq!(c[50], (92, 92), "then a sidestep across the step");
        assert_eq!(c[51], (108, 108));
        // never the start itself, never a negative coordinate
        let c = hop_candidates((3, 3), (0, -2));
        assert!(!c.contains(&(3, 3)) && c.iter().all(|&(x, y)| x >= 0 && y >= 0));
    }

    #[test]
    fn malformed_directives_name_the_problem() {
        for (line, needle) in [
            ("teleport 1 2", "unknown directive"),
            ("object 1 2", "needs 3"),
            ("superunique 1 2 3 4", "takes 3"),
            ("time 6 0", "period 6"),
            ("time 1", "takes 2"),
            ("object 1 2 3 mode 9", "object mode 9"),
            ("object 1 2 3 mode 1 mode 2", "given twice"),
            ("object 1 2 3 bogus 1", "unexpected"),
            ("seed-game 0x100000000 1", "32 bits"),
            ("seed-unit @x 1 2", "a unit is"),
            ("pos @player @z 2", "a position is"),
            ("pos @player @x+0 2", "zero offset"),
            ("pos 6/1 1 2", "unit type 6"),
            ("item HP1 1 2", "item code"),
            ("item hp1 1 2 quality best", "unknown quality"),
            ("item hp1 1 2 ilvl 100", "ilvl 100"),
            ("stat @player 0 0 2147483648", "does not fit i32"),
            ("state @player 1 maybe", "on or off"),
            ("warp 3 tile", "warp <level>"),
            ("missile 1 2 3 4 5 skill 1", "unexpected"),
            ("freeze 3601", "seconds"),
            ("goto unit 3:5", "goto unit type"),
            ("goto preset 2", "goto unit"),
            ("goto here 5", "goto unit"),
            ("goto unit 1:x", "bad number"),
            ("goto unit 2:15 mode 256", "goto mode 256"),
            ("goto superunique 40 mode 0", "goto unit"),
            ("goto preset 8 superunique 40 opening", "goto unit"),
            ("msg", "needs an id"),
            ("msg 0 1", "msg id 0"),
            ("msg 0x71", "msg id 113"),
            ("msg 0x01 @z 1", "a unit is"),
            ("msg 0x01 -1 2", "bad number"),
            ("msg 0x01 @x+0 2", "zero offset"),
            ("operate", "takes 1"),
            ("operate @wp 1", "takes 1"),
            ("operate @x", "a unit is"),
            ("talk", "needs an NPC"),
            ("talk @1:148 sell", "talk choice"),
            ("talk @1:148 quest:65536", "quest message 65536"),
            ("talk @1:148 action:x", "bad number"),
            ("", "empty"),
        ] {
            let e = parse_directive_text(line).unwrap_err();
            assert!(e.contains(needle), "{line:?}: {e}");
        }
    }

    #[test]
    fn poke_files_parse_strictly_and_round_trip() {
        let text = "# a comment\n\npoke 1\nat 1 spawn 19 @x+3 @y normal\nat 1 seed-unit @1:19 1 2 # why\nat 4 time 0x2 10\nat 4 spawn 5 1 2 unique umod 3 7\n";
        let f = PokeFile::parse(text).unwrap();
        assert_eq!(f.lines.len(), 4);
        let canon = f.to_text();
        assert_eq!(
            canon,
            "poke 1\nat 1 spawn 19 @x+3 @y normal\nat 1 seed-unit @1:19 1 2\nat 4 time 2 10\nat 4 spawn 5 1 2 unique umod 3 7\n"
        );
        assert_eq!(PokeFile::parse(&canon).unwrap(), f);
        for (bad, line, needle) in [
            ("poke 2\n", 1, "version 2"),
            ("scenario 1\n", 1, "`poke 1`"),
            (
                "poke 1\nat 5 time 1 1\nat 4 time 1 1\n",
                3,
                "after a line at tick 5",
            ),
            ("poke 1\nat 1 bogus\n", 2, "unknown directive"),
            ("poke 1\ntime 1 1\n", 2, "`at <tick>"),
            ("poke 1\nat 1 time 1\n", 2, "takes 2"),
            ("poke 1\nat 1 time 1 2 3\n", 2, "takes 2"),
            (
                "poke 1\nat 1 spawn 19 1 2 champion\n",
                2,
                "one for champion",
            ),
            ("poke 1\nat 1000001 freeze 1\n", 2, "tick"),
            ("", 0, "empty"),
        ] {
            let e = PokeFile::parse(bad).unwrap_err();
            assert_eq!(e.line, line, "{bad:?}: {e}");
            assert!(e.message.contains(needle), "{bad:?}: {e}");
        }
    }

    #[test]
    fn operate_and_talk_are_the_handler_calls_in_order() {
        let calls = |line: &str, ty, guid| {
            let d = parse_directive_text(line).unwrap();
            interact_calls_with(&d, ty, guid, 1)
                .into_iter()
                .map(|c| (c.id, c.values))
                .collect::<Vec<_>>()
        };
        assert_eq!(calls("operate @wp", 2, 18), [(0x13, vec![2, 18])]);
        assert_eq!(
            calls("talk @1:148", 1, 12),
            [(0x13, vec![1, 12]), (0x2F, vec![12])]
        );
        assert_eq!(
            calls(
                "talk @1:148 trade gamble hire action:7 quest:92 close",
                1,
                12
            ),
            [
                (0x13, vec![1, 12]),
                (0x2F, vec![12]),
                (0x38, vec![1, 12, 0]),
                (0x38, vec![2, 12, 0]),
                (0x38, vec![3, 12, 1]),
                (0x38, vec![7, 12, 0]),
                (0x31, vec![12, 92]),
                (0x30, vec![12]),
            ]
        );
        assert!(calls("freeze 1", 1, 12).is_empty());
    }

    #[test]
    fn msg_references_resolve_to_positions_and_guids() {
        // Player at (100, 200); `@1` (the first monster) has GUID 42.
        let resolve = |a: MsgArg| match a {
            MsgArg::Num(v) => Ok(i64::from(v)),
            MsgArg::Pos(Coord::X(d)) => Ok(100 + i64::from(d)),
            MsgArg::Pos(Coord::Y(d)) => Ok(200 + i64::from(d)),
            MsgArg::Pos(Coord::Num(_)) => unreachable!(),
            MsgArg::Unit(UnitArg::Nth { ty: 1, .. }) => Ok(42),
            MsgArg::Unit(u) => Err(u.to_string()),
        };
        let values = |line: &str| {
            let Directive::Msg { args, .. } = parse_directive_text(line).unwrap() else {
                panic!("{line}");
            };
            msg_values_with(&args, resolve)
        };
        assert_eq!(values("msg 0x01 @x+2 @y-201").unwrap(), [102, -1]);
        assert_eq!(values("msg 0x06 0x1 @1").unwrap(), [1, 42]);
        assert_eq!(values("msg 0x60").unwrap(), [] as [i64; 0]);
        assert_eq!(values("msg 0x06 1 @3").unwrap_err(), "@3");
        // The token decides the kind: position, unit, number.
        assert_eq!(
            parse_directive_text("msg 2 1 1/77 ").unwrap(),
            Directive::Msg {
                id: 2,
                args: vec![
                    MsgArg::Num(1),
                    MsgArg::Unit(UnitArg::Guid { ty: 1, guid: 77 })
                ],
            }
        );
    }

    #[test]
    fn results_name_their_code() {
        assert_eq!(PokeResult::Ok(Some(3)).code(), "ok");
        assert_eq!(PokeResult::Ok(Some(3)).guid(), Some(3));
        assert_eq!(PokeResult::Failed.code(), "failed");
        assert_eq!(PokeResult::FailedWith("x".into()).code(), "failed");
        assert_eq!(PokeResult::Unresolved("@1".into()).code(), "unresolved");
        assert_eq!(PokeResult::Gap("x".into()).guid(), None);
    }
}
