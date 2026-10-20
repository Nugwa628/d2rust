//! Names and checks for the repro runner: state questions answered by
//! name, both as `at T check EXPR` / `by T check EXPR` lines in a
//! scenario and as one-off queries (`repro FILE --query`, the
//! `autoplay-host` `query` command).
//!
//! Every name reads one [`state`](super::super::autoplay_host::Host::state)
//! reply of the headless client, which holds both sides of the game:
//!
//! - `snap`: the server's `state-1` snapshot (every unit's type, class,
//!   mode, position, animation frame, life and more), the authority;
//! - `client`: what the client knows and shows (its model of the player,
//!   open UI, ground items with their label text, message counts);
//! - `hud`: the numbers the HUD draws from (life, mana, stamina, gold).
//!
//! The names (the format reference is `docs/tools/scenarios.md`):
//!
//! | Name | Reads |
//! |---|---|
//! | `player.FIELD` | the local player's server unit ([`UNIT_FIELDS`]) |
//! | `unit(TYPE, CLASS).FIELD` | the first server unit of that type and class (`*`: any) |
//! | `count(TYPE, CLASS)` | how many server units match |
//! | `superunique(ROW).FIELD` | the client's monster spawned as that `superuniques` row ([`SUPERUNIQUE_FIELDS`]) |
//! | `near(TYPE, CLASS).FIELD` | the server unit of that type and class nearest the local player: a unit field, or `ax` / `ay` / `dist` ([`NEAR_FIELDS`]) |
//! | `ground_item(CODE).FIELD` | the first ground item with that code (`gold`, `*`) ([`GROUND_FIELDS`]) |
//! | `ui.open(NAME)` | whether a UI state is open ([`UI_NAMES`] or a number) |
//! | `sent(ID)`, `received(ID)` | how many C→S / S→C messages of that id |
//! | `hud.NAME` | a HUD number ([`HUD_NAMES`]) |
//! | `server.PATH`, `client.PATH` | any field of the snapshot or the client view, by path |
//! | `anim_rate(TYPE, CLASS, MODE).FIELD` | how fast the first such unit's animation was drawn in MODE (`*`: its mode now) ([`ANIM_FIELDS`]) |
//!
//! A check is `NAME`, `NAME OP VALUE` or `NAME changes`:
//!
//! - `NAME` holds when the value is true, a non-zero number or a
//!   non-empty string or list;
//! - `OP` is `==`, `!=`, `<`, `<=`, `>`, `>=`, or `~` with a tolerance
//!   (`player.pos ~ (5120, 3400) 2`: each coordinate within 2);
//! - `VALUE` is a number (`12`, `0x0C`), a quoted string, a bare word
//!   (`cast`), a list `(a, b)`, `true`, `false` or `null`;
//! - `changes` holds when the value differs from the tick before.
//!
//! A name that reads nothing (no such unit) is `null`; it equals only
//! `null` and fails every other comparison.

use serde_json::{json, Value};

/// A unit field: its name and where it comes from in a `state-1` unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// The key as stored.
    Raw(&'static str),
    /// A fixed-point 8.8 key, as whole units (life, mana, frames).
    Fixed(&'static str),
    /// Two keys as `[a, b]`.
    Pair(&'static str, &'static str),
    /// The mode, by name ([`mode_name`]).
    Mode,
    /// The Chebyshev distance to the local player, in sub-tiles.
    Distance,
}

/// The fields of `player.*` and `unit(...).*`: name, source, meaning.
pub const UNIT_FIELDS: &[(&str, Read, &str)] = &[
    ("pos", Read::Pair("x", "y"), "sub-tile position [x, y]"),
    ("x", Read::Raw("x"), "sub-tile x"),
    ("y", Read::Raw("y"), "sub-tile y"),
    ("target", Read::Pair("tx", "ty"), "path target [x, y]"),
    ("mode", Read::Mode, "mode name (walk, cast, ...)"),
    ("mode_id", Read::Raw("m"), "mode number"),
    ("dir", Read::Raw("d"), "direction"),
    ("anim_frame", Read::Fixed("fr"), "animation frame"),
    ("anim_frames", Read::Fixed("fc"), "animation frame count"),
    ("anim_speed", Read::Raw("sp"), "animation speed"),
    ("life", Read::Fixed("hp"), "life"),
    ("life_max", Read::Fixed("hpx"), "max life"),
    ("mana", Read::Fixed("mp"), "mana"),
    ("mana_max", Read::Fixed("mpx"), "max mana"),
    ("stamina", Read::Fixed("st"), "stamina"),
    ("stamina_max", Read::Fixed("stx"), "max stamina"),
    ("level", Read::Raw("lvl"), "character level"),
    ("area", Read::Raw("lv"), "level id of the unit's room"),
    ("act", Read::Raw("act"), "act"),
    ("class", Read::Raw("cl"), "class id"),
    ("guid", Read::Raw("g"), "GUID"),
    ("type", Read::Raw("ut"), "unit type"),
    ("owner", Read::Raw("own"), "owner GUID"),
    (
        "dist",
        Read::Distance,
        "Chebyshev distance to the local player, in sub-tiles",
    ),
];

/// The fields `near(TYPE, CLASS).*` has besides the unit fields: where the
/// local player stands from the unit, in sub-tiles.
pub const NEAR_FIELDS: &[(&str, &str)] = &[
    ("ax", "the player's x minus the unit's x"),
    ("ay", "the player's y minus the unit's y"),
    ("dist", "the larger of |ax| and |ay|"),
];

/// The fields of `ground_item(...).*`, read from the client view's
/// `ground` list: name, key, meaning.
pub const GROUND_FIELDS: &[(&str, &str, &str)] = &[
    ("label", "label", "the Show Items label text"),
    ("gold", "gold", "a gold pile's amount"),
    ("code", "code", "item code"),
    ("guid", "g", "GUID"),
    ("x", "x", "sub-tile x"),
    ("y", "y", "sub-tile y"),
];

/// The fields of `anim_rate(TYPE, CLASS, MODE).*`: the unit's latest
/// stretch in that mode as the world view drew it
/// (`world_view::anim_clock`), against the server's speed for the unit.
pub const ANIM_FIELDS: &[(&str, &str)] = &[
    ("fps", "drawn animation frames per second (null before `ANIM_MIN_TICKS` ticks)"),
    ("expected_fps", "the server's speed for the unit in that mode: speed / 256 frames per 25 Hz base tick (null while the server unit is in another mode)"),
    ("ratio", "fps / expected_fps: 1 when the drawn animation runs at its table rate (always 1 for a one-frame animation)"),
    ("ticks", "server ticks the stretch was drawn over"),
    ("advances", "frames the drawn picture advanced in them"),
    ("frames", "the COF's frames per direction"),
];

/// Ticks a drawn stretch needs before `anim_rate(..).fps` reads (a third
/// of a second at 60 Hz, shorter than an attack): a frame more or less then
/// moves the usual rates by about a tenth.
pub const ANIM_MIN_TICKS: u64 = 20;

/// The rate the game's speed tables are written in: speeds are 1/256
/// frames per base tick of 25 Hz.
const BASE_HZ: f64 = 25.0;

/// The `hud.*` names (keys of the state reply's `hud` object).
pub const HUD_NAMES: &[(&str, &str)] = &[
    ("life", "life the globe and its number show"),
    ("life_max", "max life"),
    ("mana", "mana the globe and its number show"),
    ("mana_max", "max mana"),
    ("stamina", "stamina the bar shows"),
    ("stamina_max", "max stamina"),
    ("gold", "gold carried"),
    ("level", "character level"),
    ("exp", "experience"),
];

/// The fields of `superunique(ROW)`: the client's `superuniques` list.
pub const SUPERUNIQUE_FIELDS: &[(&str, &str)] = &[
    ("guid", "the monster's GUID"),
    ("class", "its monstats row"),
    ("mode", "mode name (neutral, death, dead, ...)"),
    ("pos", "sub-tile position [x, y]"),
    ("dead", "true once it is dying or dead"),
    (
        "dist",
        "Chebyshev distance to the local player, in sub-tiles",
    ),
    ("life", "whole life points of the server unit with its GUID"),
    (
        "life_max",
        "whole max life of the server unit with its GUID",
    ),
];

/// The UI states `ui.open(NAME)` knows by name.
pub const UI_NAMES: &[(&str, u8)] = &[
    ("inventory", 0x01),
    ("character", 0x02),
    ("skill_select", 0x03),
    ("skill_tree", 0x04),
    ("chat", 0x05),
    ("new_stats", 0x06),
    ("new_skills", 0x07),
    ("npc_menu", 0x08),
    ("esc_menu", 0x09),
    ("automap", 0x0A),
    ("npc_shop", 0x0C),
    ("show_items", 0x0D),
    ("socket", 0x0E),
    ("quest_screen", 0x0F),
    ("quest_log", 0x11),
    ("hireling_icons", 0x13),
    ("waypoint", 0x14),
    ("mini_panel", 0x15),
    ("party", 0x16),
    ("message_log", 0x18),
    ("stash", 0x19),
    ("cube", 0x1A),
    ("hireling_inventory", 0x24),
];

/// The unit types `unit(TYPE, ...)` knows by name.
const UNIT_TYPES: &[(&str, u64)] = &[
    ("player", 0),
    ("monster", 1),
    ("object", 2),
    ("missile", 3),
    ("item", 4),
    ("tile", 5),
];

/// A player's modes by number.
const PLAYER_MODES: &[&str] = &[
    "death",
    "neutral",
    "walk",
    "run",
    "gethit",
    "town_neutral",
    "town_walk",
    "attack1",
    "attack2",
    "block",
    "cast",
    "throw",
    "kick",
    "skill1",
    "skill2",
    "skill3",
    "skill4",
    "dead",
    "sequence",
    "knockback",
];

/// A monster's modes by number.
const MONSTER_MODES: &[&str] = &[
    "death",
    "neutral",
    "walk",
    "gethit",
    "attack1",
    "attack2",
    "block",
    "cast",
    "skill1",
    "skill2",
    "skill3",
    "skill4",
    "dead",
    "knockback",
    "sequence",
    "run",
];

/// The name of mode `m` of a unit of type `ut`: players and monsters by
/// name, other types by number.
pub fn mode_name(ut: u64, m: u64) -> Value {
    let table = match ut {
        0 => PLAYER_MODES,
        1 => MONSTER_MODES,
        _ => return json!(m),
    };
    usize::try_from(m)
        .ok()
        .and_then(|i| table.get(i))
        .map_or_else(|| json!(m), |n| json!(n))
}

/// A parsed name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    /// The name as written (spaces removed), the key of a query's reply.
    pub text: String,
    kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    /// The local player's server unit.
    Player(Read),
    /// The first server unit of a type and class (`None`: any class).
    Unit(u64, Option<u64>, Read),
    Count(u64, Option<u64>),
    /// A field of the client's superunique monster of a `superuniques` row.
    Superunique(u64, &'static str),
    /// The first ground item with a code (`None`: any) and a field key.
    Ground(Option<String>, &'static str),
    UiOpen(u8),
    /// A quest flag bit: slot and bit of the local player's record.
    Quest(u8, u8),
    /// A quest flag slot's word of the local player's record.
    QuestWord(u8),
    Sent(u8),
    Received(u8),
    /// The drawn animation rate of the first unit of a type and class, in
    /// a mode (`None`: the mode it is drawn in now), and a field of
    /// [`ANIM_FIELDS`].
    AnimRate(u64, Option<u64>, Option<u64>, &'static str),
    /// The unit of a type and class nearest the local player: a unit field,
    /// or (`Err`) a field of [`NEAR_FIELDS`].
    Near(u64, Option<u64>, Result<Read, &'static str>),
    /// A path in the reply: the top key (`snap`, `client`, `hud`), then
    /// keys or list indices.
    Path(&'static str, Vec<String>),
}

/// Splits `s` on `sep` outside parentheses and quotes.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut depth, mut quoted, mut from) = (0i32, false, 0);
    let mut out = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth -= 1,
            c if c == sep && !quoted && depth == 0 => {
                out.push(&s[from..i]);
                from = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[from..]);
    out
}

/// `head(a, b)` → (`head`, [`a`, `b`]); a plain word → (word, []).
fn call(seg: &str) -> Result<(&str, Vec<&str>), String> {
    match seg.split_once('(') {
        None => Ok((seg, Vec::new())),
        Some((head, rest)) => {
            let args = rest
                .strip_suffix(')')
                .ok_or_else(|| format!("`{seg}`: missing `)`"))?;
            Ok((head, args.split(',').map(str::trim).collect()))
        }
    }
}

fn number(s: &str) -> Option<u64> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

fn byte(s: &str, what: &str) -> Result<u8, String> {
    number(s)
        .and_then(|n| u8::try_from(n).ok())
        .ok_or_else(|| format!("{what} `{s}`: a number 0-255 (`0x0C` or `12`)"))
}

/// The one field after a name's head.
fn one_field<'a>(rest: &[&'a str], text: &str, what: &str) -> Result<&'a str, String> {
    match rest {
        [f] => Ok(f),
        _ => Err(format!("`{text}`: `{what}.FIELD`")),
    }
}

fn unit_field(f: &str) -> Result<Read, String> {
    UNIT_FIELDS
        .iter()
        .find(|(n, _, _)| *n == f)
        .map(|(_, r, _)| *r)
        .ok_or_else(|| {
            let all: Vec<&str> = UNIT_FIELDS.iter().map(|(n, _, _)| *n).collect();
            format!("unknown unit field `{f}`: {}", all.join(", "))
        })
}

fn unit_args(args: &[&str]) -> Result<(u64, Option<u64>), String> {
    let [ty, class] = args else {
        return Err("`unit(TYPE, CLASS)`: two arguments".into());
    };
    let ty = UNIT_TYPES
        .iter()
        .find(|(n, _)| n == ty)
        .map(|(_, t)| *t)
        .or_else(|| number(ty).filter(|t| *t <= 5))
        .ok_or_else(|| format!("unit type `{ty}`: player, monster, object, missile, item, tile"))?;
    let class = match *class {
        "*" => None,
        c => Some(number(c).ok_or_else(|| format!("unit class `{c}`: a number or `*`"))?),
    };
    Ok((ty, class))
}

/// Parses one name.
pub fn parse_name(s: &str) -> Result<Name, String> {
    let text: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if text.is_empty() {
        return Err("an empty name".into());
    }
    let segs = split_top(&text, '.');
    let (head, args) = call(segs[0])?;
    let rest = &segs[1..];
    let field = |what: &str| one_field(rest, &text, what);
    let kind = match (head, args.is_empty()) {
        ("player", true) => Kind::Player(unit_field(field("player")?)?),
        ("unit", false) => {
            let (ty, class) = unit_args(&args)?;
            Kind::Unit(ty, class, unit_field(field("unit(TYPE, CLASS)")?)?)
        }
        ("near", false) => {
            let (ty, class) = unit_args(&args)?;
            let f = field("near(TYPE, CLASS)")?;
            let read = match NEAR_FIELDS.iter().find(|(n, _)| *n == f) {
                Some((n, _)) => Err(*n),
                None => Ok(unit_field(f).map_err(|e| format!("{e}, ax, ay, dist"))?),
            };
            Kind::Near(ty, class, read)
        }
        ("count", false) if rest.is_empty() => {
            let (ty, class) = unit_args(&args)?;
            Kind::Count(ty, class)
        }
        ("ground_item", false) => {
            let code = match args.as_slice() {
                ["*"] => None,
                ["gold"] => Some("gld".to_owned()),
                [c] if (1..=4).contains(&c.len()) => Some((*c).to_owned()),
                _ => return Err("`ground_item(CODE)`: one item code, `gold` or `*`".into()),
            };
            let f = field("ground_item(CODE)")?;
            let key = GROUND_FIELDS
                .iter()
                .find(|(n, _, _)| *n == f)
                .map(|(_, k, _)| *k)
                .ok_or_else(|| {
                    let all: Vec<&str> = GROUND_FIELDS.iter().map(|(n, _, _)| *n).collect();
                    format!("unknown ground item field `{f}`: {}", all.join(", "))
                })?;
            Kind::Ground(code, key)
        }
        ("superunique", false) => {
            let [row] = args.as_slice() else {
                return Err("`superunique(ROW)`: one superuniques row".into());
            };
            let row = number(row).ok_or_else(|| format!("superuniques row `{row}`: a number"))?;
            let f = field("superunique(ROW)")?;
            let (n, _) = SUPERUNIQUE_FIELDS
                .iter()
                .find(|(n, _)| *n == f)
                .ok_or_else(|| {
                    let all: Vec<&str> = SUPERUNIQUE_FIELDS.iter().map(|(n, _)| *n).collect();
                    format!("unknown superunique field `{f}`: {}", all.join(", "))
                })?;
            Kind::Superunique(row, n)
        }
        ("ui", true) => match rest {
            [open] => {
                let (h, a) = call(open)?;
                match (h, a.as_slice()) {
                    ("open", [n]) => Kind::UiOpen(
                        UI_NAMES
                            .iter()
                            .find(|(k, _)| k == n)
                            .map(|(_, id)| *id)
                            .map_or_else(|| byte(n, "ui state"), Ok)?,
                    ),
                    _ => return Err(format!("`{text}`: `ui.open(NAME)`")),
                }
            }
            _ => return Err(format!("`{text}`: `ui.open(NAME)`")),
        },
        ("quest", false) if rest.is_empty() => {
            let [slot, bit] = args.as_slice() else {
                return Err("`quest(SLOT, BIT)`: two numbers".into());
            };
            Kind::Quest(byte(slot, "quest slot")?, byte(bit, "quest bit")?)
        }
        ("quest_word", false) if rest.is_empty() => {
            let [slot] = args.as_slice() else {
                return Err("`quest_word(SLOT)`: one number".into());
            };
            Kind::QuestWord(byte(slot, "quest slot")?)
        }
        ("sent", false) | ("received", false) if rest.is_empty() => {
            let [id] = args.as_slice() else {
                return Err(format!("`{head}(ID)`: one message id"));
            };
            let id = byte(id, "message id")?;
            if head == "sent" {
                Kind::Sent(id)
            } else {
                Kind::Received(id)
            }
        }
        ("anim_rate", false) => {
            let [ty, class, mode] = args.as_slice() else {
                return Err("`anim_rate(TYPE, CLASS, MODE)`: three arguments".into());
            };
            let (ty, class) = unit_args(&[ty, class])?;
            let table = match ty {
                0 => PLAYER_MODES,
                1 => MONSTER_MODES,
                _ => &[][..],
            };
            let name = *mode;
            let mode = (name != "*").then(|| {
                table
                    .iter()
                    .position(|m| m == mode)
                    .map(|m| m as u64)
                    .or_else(|| number(mode))
            });
            let mode = mode.map_or(Some(None), |m| m.map(Some)).ok_or_else(|| {
                format!("mode `{name}`: a mode name of that unit type, a number or `*`")
            })?;
            let f = field("anim_rate(TYPE, CLASS, MODE)")?;
            let (f, _) = ANIM_FIELDS.iter().find(|(n, _)| *n == f).ok_or_else(|| {
                let all: Vec<&str> = ANIM_FIELDS.iter().map(|(n, _)| *n).collect();
                format!("unknown anim_rate field `{f}`: {}", all.join(", "))
            })?;
            Kind::AnimRate(ty, class, mode, f)
        }
        ("hud", true) => {
            let f = field("hud")?;
            let (n, _) = HUD_NAMES.iter().find(|(n, _)| *n == f).ok_or_else(|| {
                let all: Vec<&str> = HUD_NAMES.iter().map(|(n, _)| *n).collect();
                format!("unknown hud name `{f}`: {}", all.join(", "))
            })?;
            Kind::Path("hud", vec![(*n).to_owned()])
        }
        ("server", true) | ("client", true) if !rest.is_empty() => Kind::Path(
            if head == "server" { "snap" } else { "client" },
            rest.iter().map(|s| (*s).to_owned()).collect(),
        ),
        _ => {
            return Err(format!(
                "unknown name `{text}`: player.F, unit(T, C).F, count(T, C), \
                 superunique(ROW).F, near(T, C).F, ground_item(CODE).F, ui.open(N), sent(ID), \
                 received(ID), hud.N, anim_rate(T, C, MODE).F, server.PATH or client.PATH"
            ))
        }
    };
    Ok(Name { text, kind })
}

/// Parses a query: names separated by commas.
pub fn parse_names(s: &str) -> Result<Vec<Name>, String> {
    split_top(s, ',').into_iter().map(parse_name).collect()
}

/// `v[key]`, or `v[index]` for a number on a list.
fn step<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Array(a) => key.parse::<usize>().ok().and_then(|i| a.get(i)),
        _ => v.get(key),
    }
}

/// A unit's sub-tile position: the snapshot's `x`/`y` keys, or the client
/// view's `pos` pair.
fn position(u: &Value) -> Option<(i64, i64)> {
    if let Some(p) = u.get("pos").and_then(Value::as_array) {
        return Some((p.first()?.as_i64()?, p.get(1)?.as_i64()?));
    }
    Some((u.get("x")?.as_i64()?, u.get("y")?.as_i64()?))
}

/// The Chebyshev distance between two units, `null` when either has no
/// position.
fn distance(a: &Value, b: &Value) -> Value {
    match (position(a), position(b)) {
        (Some((ax, ay)), Some((bx, by))) => json!((ax - bx).abs().max((ay - by).abs())),
        _ => Value::Null,
    }
}

fn local_unit(state: &Value) -> Option<&Value> {
    let local = state["local"].as_u64()?;
    units(state)
        .iter()
        .find(|u| u["ut"].as_u64() == Some(0) && u["g"].as_u64() == Some(local))
}

fn read_unit(u: &Value, r: Read, local: Option<&Value>) -> Value {
    let get = |k: &str| u.get(k).cloned().unwrap_or(Value::Null);
    match r {
        Read::Raw(k) => get(k),
        Read::Fixed(k) => u
            .get(k)
            .and_then(Value::as_i64)
            .map_or(Value::Null, |v| json!(v >> 8)),
        Read::Pair(a, b) => match (u.get(a), u.get(b)) {
            (Some(a), Some(b)) => json!([a, b]),
            _ => Value::Null,
        },
        Read::Mode => match (
            u.get("ut").and_then(Value::as_u64),
            u.get("m").and_then(Value::as_u64),
        ) {
            (Some(ut), Some(m)) => mode_name(ut, m),
            _ => Value::Null,
        },
        Read::Distance => local.map_or(Value::Null, |l| distance(u, l)),
    }
}

fn units(state: &Value) -> &[Value] {
    state["snap"]["units"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
}

/// The local player's quest flag word in `slot`, from the snapshot's `q`.
fn quest_word(state: &Value, slot: u8) -> Option<u16> {
    let local = state["local"].as_u64()?;
    let u = units(state)
        .iter()
        .find(|u| u["ut"].as_u64() == Some(0) && u["g"].as_u64() == Some(local))?;
    u["q"]
        .as_array()?
        .iter()
        .find(|pair| pair[0].as_u64() == Some(u64::from(slot)))
        .and_then(|pair| pair[1].as_u64())
        .and_then(|word| u16::try_from(word).ok())
}

fn matches(u: &Value, ty: u64, class: Option<u64>) -> bool {
    u["ut"].as_u64() == Some(ty) && class.is_none_or(|c| u["cl"].as_u64() == Some(c))
}

/// `anim_rate(TYPE, CLASS, MODE).FIELD` in one state reply.
fn anim_rate(state: &Value, ty: u64, class: Option<u64>, mode: Option<u64>, field: &str) -> Value {
    let Some(entry) = state["client"]["anim"].as_array().and_then(|a| {
        a.iter().find(|u| {
            u["t"].as_u64() == Some(ty) && class.is_none_or(|c| u["class"].as_u64() == Some(c))
        })
    }) else {
        return Value::Null;
    };
    let Some(mode) = mode.or_else(|| entry["mode"].as_u64()) else {
        return Value::Null;
    };
    let seg = &entry["modes"][mode.to_string()];
    if seg.is_null() {
        return Value::Null;
    }
    let ticks = seg["ticks"].as_u64().unwrap_or(0);
    let fps = seg["fps"].as_f64().filter(|_| ticks >= ANIM_MIN_TICKS);
    let expected = units(state)
        .iter()
        .find(|u| {
            u["ut"].as_u64() == Some(ty) && u["g"] == entry["g"] && u["m"].as_u64() == Some(mode)
        })
        .and_then(|u| u["sp"].as_f64())
        .map(|sp| sp / 256.0 * BASE_HZ);
    let round = |v: f64| (v * 1000.0).round() / 1000.0;
    match field {
        "fps" => fps.map_or(Value::Null, |v| json!(round(v))),
        "expected_fps" => expected.map_or(Value::Null, |v| json!(round(v))),
        "ratio" => match (fps, expected) {
            // One frame per direction draws the same picture at any rate.
            (Some(_), Some(_)) if seg["frames"].as_u64().is_some_and(|n| n <= 1) => json!(1.0),
            (Some(f), Some(e)) if e > 0.0 => json!(round(f / e)),
            _ => Value::Null,
        },
        "ticks" => json!(ticks),
        "advances" => seg["advances"].clone(),
        _ => seg["frames"].clone(),
    }
}

/// `near(TYPE, CLASS).FIELD`: the matching unit nearest the local player
/// (squared distance, ties by GUID), with the player's offset from it.
fn nearest_unit(state: &Value, ty: u64, class: Option<u64>, r: Result<Read, &str>) -> Value {
    let local = state["local"].as_u64();
    let pos = |u: &Value| Some((u["x"].as_i64()?, u["y"].as_i64()?));
    let Some(me) = units(state)
        .iter()
        .find(|u| u["ut"].as_u64() == Some(0) && local.is_some() && u["g"].as_u64() == local)
        .and_then(pos)
    else {
        return Value::Null;
    };
    let Some((u, (x, y))) = units(state)
        .iter()
        .filter(|u| matches(u, ty, class))
        .filter_map(|u| pos(u).map(|p| (u, p)))
        .min_by_key(|(u, (x, y))| {
            let (dx, dy) = (x - me.0, y - me.1);
            (dx * dx + dy * dy, u["g"].as_u64().unwrap_or(0))
        })
    else {
        return Value::Null;
    };
    let (ax, ay) = (me.0 - x, me.1 - y);
    match r {
        Ok(read) => read_unit(u, read, local_unit(state)),
        Err("ax") => json!(ax),
        Err("ay") => json!(ay),
        Err(_) => json!(ax.abs().max(ay.abs())),
    }
}

fn count_of(map: &Value, id: u8) -> Value {
    json!(map[format!("{id:02X}")].as_u64().unwrap_or(0))
}

impl Name {
    /// The name's value in one `state` reply (`null` when it reads
    /// nothing).
    pub fn eval(&self, state: &Value) -> Value {
        match &self.kind {
            Kind::Player(r) => local_unit(state).map_or(Value::Null, |u| read_unit(u, *r, Some(u))),
            Kind::Unit(ty, class, r) => units(state)
                .iter()
                .find(|u| matches(u, *ty, *class))
                .map_or(Value::Null, |u| read_unit(u, *r, local_unit(state))),
            Kind::Near(ty, class, r) => nearest_unit(state, *ty, *class, *r),
            Kind::Count(ty, class) => {
                json!(units(state)
                    .iter()
                    .filter(|u| matches(u, *ty, *class))
                    .count())
            }
            Kind::Superunique(row, f) => state["client"]["superuniques"]
                .as_array()
                .and_then(|l| l.iter().find(|u| u["row"].as_u64() == Some(*row)))
                .map_or(Value::Null, |u| match *f {
                    "guid" => u["g"].clone(),
                    "mode" => u["mode"].as_u64().map_or(Value::Null, |m| mode_name(1, m)),
                    "dead" => json!(
                        u["g"].as_u64().is_some_and(|g| g != 0)
                            && matches!(u["mode"].as_u64(), Some(0 | 12))
                    ),
                    "dist" => local_unit(state).map_or(Value::Null, |l| distance(u, l)),
                    // The server unit with the row's GUID, not the first of
                    // its class (its minions often share the class).
                    "life" | "life_max" => units(state)
                        .iter()
                        .find(|s| s["ut"].as_u64() == Some(1) && s["g"] == u["g"])
                        .map_or(Value::Null, |s| {
                            let k = if *f == "life" { "hp" } else { "hpx" };
                            read_unit(s, Read::Fixed(k), None)
                        }),
                    f => u[f].clone(),
                }),
            Kind::Ground(code, key) => state["client"]["ground"]
                .as_array()
                .and_then(|g| {
                    g.iter().find(|i| {
                        code.as_deref()
                            .is_none_or(|c| i["code"].as_str() == Some(c))
                    })
                })
                .map_or(Value::Null, |i| i.get(*key).cloned().unwrap_or(Value::Null)),
            Kind::UiOpen(id) => json!(state["client"]["open"]
                .as_array()
                .is_some_and(|o| o.iter().any(|v| v.as_u64() == Some(u64::from(*id))))),
            Kind::Quest(slot, bit) => {
                json!(quest_word(state, *slot).is_some_and(|w| w & (1 << u16::from(*bit & 15)) != 0))
            }
            Kind::QuestWord(slot) => json!(quest_word(state, *slot).unwrap_or(0)),
            Kind::Sent(id) => count_of(&state["client"]["sent"], *id),
            Kind::AnimRate(ty, class, mode, field) => anim_rate(state, *ty, *class, *mode, field),
            Kind::Received(id) => count_of(&state["client"]["received"], *id),
            Kind::Path(top, path) => path
                .iter()
                .try_fold(&state[*top], |v, k| step(v, k))
                .cloned()
                .unwrap_or(Value::Null),
        }
    }
}

/// The values of `names` in one `state` reply, by name.
pub fn query(names: &[Name], state: &Value) -> Value {
    Value::Object(
        names
            .iter()
            .map(|n| (n.text.clone(), n.eval(state)))
            .collect(),
    )
}

/// A comparison.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `~ VALUE N`: every number within N.
    Near(f64),
}

/// A check expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    /// `NAME`: the value is truthy.
    Holds(Name),
    /// `NAME changes`: the value differs from the tick before.
    Changes(Name),
    /// `NAME OP VALUE`.
    Cmp(Name, Op, Value),
}

/// Parses a literal value.
fn literal(s: &str) -> Result<Value, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("a comparison needs a value".into());
    }
    if let Some(q) = s.strip_prefix('"') {
        return q
            .strip_suffix('"')
            .map(|t| json!(t))
            .ok_or_else(|| format!("`{s}`: missing closing quote"));
    }
    if let Some(inner) = s.strip_prefix('(') {
        let inner = inner
            .strip_suffix(')')
            .ok_or_else(|| format!("`{s}`: missing `)`"))?;
        return split_top(inner, ',')
            .into_iter()
            .map(literal)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    Ok(match s {
        "true" => json!(true),
        "false" => json!(false),
        "null" => Value::Null,
        _ => {
            let neg = s.strip_prefix('-');
            if let Some(n) = number(neg.unwrap_or(s)) {
                let n = i64::try_from(n).map_err(|_| format!("`{s}`: too large"))?;
                json!(if neg.is_some() { -n } else { n })
            } else if let Ok(f) = s.parse::<f64>() {
                json!(f)
            } else if s.chars().all(|c| c.is_alphanumeric() || c == '_') {
                json!(s)
            } else {
                return Err(format!(
                    "`{s}`: not a value (a number, \"text\", word or (a, b))"
                ));
            }
        }
    })
}

/// Finds the first operator outside parentheses and quotes.
fn find_op(s: &str) -> Option<(usize, &'static str)> {
    let (mut depth, mut quoted) = (0i32, false);
    let b = s.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'"' => quoted = !quoted,
            b'(' if !quoted => depth += 1,
            b')' if !quoted => depth -= 1,
            _ if quoted || depth != 0 || !s.is_char_boundary(i) => {}
            _ => {
                for op in ["==", "!=", "<=", ">=", "<", ">", "~"] {
                    if s[i..].starts_with(op) {
                        return Some((i, op));
                    }
                }
            }
        }
    }
    None
}

/// Parses a check expression.
pub fn parse_check(s: &str) -> Result<Check, String> {
    let s = s.trim();
    if let Some((i, op)) = find_op(s) {
        let name = parse_name(&s[..i])?;
        let rhs = s[i + op.len()..].trim();
        let op = match op {
            "==" => Op::Eq,
            "!=" => Op::Ne,
            "<" => Op::Lt,
            "<=" => Op::Le,
            ">" => Op::Gt,
            ">=" => Op::Ge,
            _ => {
                // `~ VALUE N`: the tolerance is the last word after the value.
                let (value, tol) = match rhs.rsplit_once(char::is_whitespace) {
                    Some((v, t)) if !v.trim().is_empty() && t.parse::<f64>().is_ok() => {
                        (v, t.parse::<f64>().unwrap_or(0.0))
                    }
                    _ => (rhs, 0.0),
                };
                return Ok(Check::Cmp(name, Op::Near(tol), literal(value)?));
            }
        };
        return Ok(Check::Cmp(name, op, literal(rhs)?));
    }
    if let Some(n) = s.strip_suffix("changes") {
        if n.ends_with(char::is_whitespace) {
            return Ok(Check::Changes(parse_name(n)?));
        }
    }
    Ok(Check::Holds(parse_name(s)?))
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(_) => true,
    }
}

/// `a == b`, numbers by value (`5` equals `5.0`).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b))
        }
        _ => a == b,
    }
}

fn near(a: &Value, b: &Value, tol: f64) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= tol,
            _ => false,
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| near(a, b, tol))
        }
        _ => false,
    }
}

impl Check {
    /// The name the check reads.
    pub fn name(&self) -> &Name {
        match self {
            Self::Holds(n) | Self::Changes(n) | Self::Cmp(n, _, _) => n,
        }
    }

    /// Whether the check holds in `now` (with `prev` the reply of the
    /// tick before, for `changes`), and the value it read.
    pub fn eval(&self, now: &Value, prev: Option<&Value>) -> (bool, Value) {
        let v = self.name().eval(now);
        let ok = match self {
            Self::Holds(_) => truthy(&v),
            Self::Changes(n) => prev.is_some_and(|p| !same(&n.eval(p), &v)),
            Self::Cmp(_, op, want) => {
                let ord = || match (v.as_f64(), want.as_f64()) {
                    (Some(a), Some(b)) => a.partial_cmp(&b),
                    _ => None,
                };
                use std::cmp::Ordering::*;
                match op {
                    Op::Eq => same(&v, want),
                    Op::Ne => !same(&v, want),
                    Op::Lt => ord() == Some(Less),
                    Op::Le => matches!(ord(), Some(Less | Equal)),
                    Op::Gt => ord() == Some(Greater),
                    Op::Ge => matches!(ord(), Some(Greater | Equal)),
                    Op::Near(t) => near(&v, want, *t),
                }
            }
        };
        (ok, v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `state` reply with a player (GUID 1), two monsters, a gold pile
    /// and some client and HUD values.
    fn state() -> Value {
        json!({
            "local": 1,
            "snap": {"f": 120, "units": [
                {"ut": 0, "g": 1, "cl": 1, "m": 10, "x": 5120, "y": 3400, "fr": 1280, "fc": 4096,
                 "hp": 37376, "hpx": 37376, "lvl": 30, "lv": 3},
                {"ut": 1, "g": 7, "cl": 19, "m": 2, "x": 5130, "y": 3404, "fr": 512, "sp": 128},
                {"ut": 1, "g": 9, "cl": 20, "m": 1, "hp": 25600, "hpx": 37376},
            ]},
            "client": {
                "open": [1, 13],
                "ground": [
                    {"g": 30, "code": "hp1", "x": 1, "y": 2, "gold": null, "label": "Minor Healing Potion"},
                    {"g": 31, "code": "gld", "x": 3, "y": 4, "gold": 1234, "label": "1234 Gold"},
                ],
                "sent": {"0C": 2},
                "received": {"15": 40},
                "player": {"pos": [5121, 3401]},
                "superuniques": [
                    {"g": 40, "row": 0, "class": 58, "mode": 1, "pos": [5140, 3410]},
                    {"g": 41, "row": 40, "class": 2, "mode": 12, "pos": [5150, 3420]},
                    {"g": 9, "row": 7, "class": 20, "mode": 1},
                ],
                "anim": [{"t": 1, "g": 7, "class": 19, "mode": 2, "frame": 3, "frames": 8, "modes": {
                    "2": {"frames": 8, "ticks": 60, "advances": 25, "fps": 25.0},
                    "1": {"frames": 8, "ticks": 10, "advances": 4, "fps": 24.0},
                }}],
            },
            "hud": {"life": 146, "life_max": 146, "mana": 50},
        })
    }

    fn eval(name: &str) -> Value {
        parse_name(name).unwrap().eval(&state())
    }

    #[test]
    fn server_names_read_the_snapshot() {
        assert_eq!(eval("player.pos"), json!([5120, 3400]));
        assert_eq!(eval("player.mode"), json!("cast"));
        assert_eq!(eval("player.mode_id"), json!(10));
        assert_eq!(eval("player.anim_frame"), json!(5));
        assert_eq!(eval("player.life"), json!(146));
        assert_eq!(eval("player.level"), json!(30));
        assert_eq!(eval("unit(monster, *).mode"), json!("walk"));
        assert_eq!(eval("unit(monster, 20).mode"), json!("neutral"));
        assert_eq!(eval("unit(1, 20).guid"), json!(9));
        assert_eq!(eval("unit(monster, 21).mode"), Value::Null);
        assert_eq!(eval("player.dist"), json!(0));
        assert_eq!(eval("unit(monster, 19).dist"), json!(10));
        assert_eq!(eval("unit(monster, 20).dist"), Value::Null);
        assert_eq!(eval("count(monster, *)"), json!(2));
        // The player stands at (5120, 3400); monster 7 at (5130, 3404).
        assert_eq!(eval("near(monster, *).guid"), json!(7));
        assert_eq!(eval("near(monster, *).ax"), json!(-10));
        assert_eq!(eval("near(monster, *).ay"), json!(-4));
        assert_eq!(eval("near(monster, *).dist"), json!(10));
        assert_eq!(eval("near(monster, 20).guid"), Value::Null);
        assert!(parse_name("near(monster, *).side").is_err());
        assert_eq!(eval("server.f"), json!(120));
        assert_eq!(eval("server.units.1.g"), json!(7));
    }

    #[test]
    fn a_superunique_is_found_by_its_row_not_its_class() {
        assert_eq!(eval("superunique(0).guid"), json!(40));
        assert_eq!(eval("superunique(0).mode"), json!("neutral"));
        assert_eq!(eval("superunique(0).dead"), json!(false));
        assert_eq!(eval("superunique(40).dead"), json!(true));
        assert_eq!(eval("superunique(40).class"), json!(2));
        assert_eq!(eval("superunique(40).pos"), json!([5150, 3420]));
        assert_eq!(eval("superunique(0).dist"), json!(20));
        assert_eq!(eval("superunique(3).guid"), Value::Null);
        // Life is read from the server unit with the row's GUID (9); a row
        // with no server unit (GUID 41 here) reads nothing.
        assert_eq!(eval("superunique(7).life"), json!(100));
        assert_eq!(eval("superunique(7).life_max"), json!(146));
        assert_eq!(eval("superunique(40).life"), Value::Null);
        assert!(parse_name("superunique(0).hp").is_err());
        assert!(parse_name("superunique(0)").is_err());
    }

    #[test]
    fn anim_rate_compares_the_drawn_rate_with_the_server_speed() {
        // Drawn at 25 frames a second; the server's speed 128 is 12.5.
        assert_eq!(eval("anim_rate(monster, 19, walk).fps"), json!(25.0));
        assert_eq!(
            eval("anim_rate(monster, 19, walk).expected_fps"),
            json!(12.5)
        );
        assert_eq!(eval("anim_rate(monster, *, 2).ratio"), json!(2.0));
        assert_eq!(eval("anim_rate(monster, 19, walk).advances"), json!(25));
        // `*`: the mode the unit is drawn in now (walk).
        assert_eq!(eval("anim_rate(monster, 19, *).ratio"), json!(2.0));
        // Too short a stretch reads no rate; the server unit is walking,
        // so there is no speed for neutral either.
        assert_eq!(eval("anim_rate(monster, 19, neutral).fps"), Value::Null);
        assert_eq!(eval("anim_rate(monster, 19, neutral).ticks"), json!(10));
        assert_eq!(eval("anim_rate(monster, 19, neutral).ratio"), Value::Null);
        assert_eq!(eval("anim_rate(monster, 20, walk).fps"), Value::Null);
        assert_eq!(eval("anim_rate(player, *, walk).fps"), Value::Null);
        for bad in [
            "anim_rate(monster, *).fps",
            "anim_rate(monster, *, walk).speed",
            "anim_rate(monster, *, flying).fps",
        ] {
            assert!(parse_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_one_frame_animation_is_at_its_rate_whatever_its_speed() {
        // A bone prison stands in its one-frame neutral: nothing advances.
        let mut s = state();
        s["snap"]["units"] = json!([{"ut": 1, "g": 37, "cl": 340, "m": 1, "sp": 256}]);
        s["client"]["anim"] = json!([{"t": 1, "g": 37, "class": 340, "mode": 1, "frame": 0,
            "frames": 1, "modes": {"1": {"frames": 1, "ticks": 300, "advances": 0, "fps": 0.0}}}]);
        let read = |n: &str| parse_name(n).unwrap().eval(&s);
        assert_eq!(read("anim_rate(monster, 340, *).fps"), json!(0.0));
        assert_eq!(read("anim_rate(monster, 340, *).ratio"), json!(1.0));
        // A stretch too short still reads no rate.
        s["client"]["anim"][0]["modes"]["1"]["ticks"] = json!(5);
        assert_eq!(
            parse_name("anim_rate(monster, 340, *).ratio")
                .unwrap()
                .eval(&s),
            Value::Null
        );
    }

    #[test]
    fn client_and_hud_names_read_the_client_view() {
        assert_eq!(eval("ground_item(gold).label"), json!("1234 Gold"));
        assert_eq!(eval("ground_item(gld).gold"), json!(1234));
        assert_eq!(eval("ground_item(*).code"), json!("hp1"));
        assert_eq!(eval("ground_item(rvl).label"), Value::Null);
        assert_eq!(eval("ui.open(inventory)"), json!(true));
        assert_eq!(eval("ui.open(show_items)"), json!(true));
        assert_eq!(eval("ui.open(0x02)"), json!(false));
        assert_eq!(eval("sent(0x0C)"), json!(2));
        assert_eq!(eval("sent(0x0D)"), json!(0));
        assert_eq!(eval("received(0x15)"), json!(40));
        assert_eq!(eval("hud.life"), json!(146));
        assert_eq!(eval("hud.stamina"), Value::Null);
        assert_eq!(eval("client.player.pos.0"), json!(5121));
    }

    #[test]
    fn bad_names_are_refused_with_the_choices() {
        for bad in [
            "",
            "player",
            "player.speed",
            "unit(monster).mode",
            "unit(ghost, *).mode",
            "ground_item(gold).weight",
            "ui.open",
            "sent(0x100)",
            "hud.armor",
            "server",
            "nothing.here",
        ] {
            assert!(parse_name(bad).is_err(), "{bad:?} parsed");
        }
        let e = parse_name("player.speed").unwrap_err();
        assert!(e.contains("pos") && e.contains("anim_frame"), "{e}");
    }

    #[test]
    fn queries_split_on_top_level_commas() {
        let names = parse_names("player.pos, unit(monster, *).mode, hud.life").unwrap();
        assert_eq!(names.len(), 3);
        assert_eq!(names[1].text, "unit(monster,*).mode");
        assert_eq!(
            query(&names, &state()),
            json!({"player.pos": [5120, 3400], "unit(monster,*).mode": "walk", "hud.life": 146})
        );
    }

    fn check(expr: &str) -> (bool, Value) {
        parse_check(expr).unwrap().eval(&state(), None)
    }

    #[test]
    fn comparisons() {
        assert_eq!(check("player.mode == cast"), (true, json!("cast")));
        assert!(check("player.mode != walk").0);
        assert!(check("ground_item(gold).label == \"1234 Gold\"").0);
        assert_eq!(
            check("ground_item(gold).label == \"Gold\""),
            (false, json!("1234 Gold"))
        );
        assert!(check("hud.life == 146").0);
        assert!(check("sent(0x0C) >= 1").0);
        assert!(!check("sent(0x0C) > 2").0);
        assert!(check("sent(0x0C) <= 2").0);
        assert!(check("player.life < 0x100").0);
        assert!(check("player.pos == (5120, 3400)").0);
        assert!(check("player.pos ~ (5121,3398) 2").0);
        assert_eq!(
            check("player.pos ~ (5123, 3400) 2"),
            (false, json!([5120, 3400]))
        );
        assert!(check("player.x ~ 5120").0);
        assert!(check("unit(monster, 21).mode == null").0);
        // A missing value fails every comparison but `== null`.
        assert_eq!(check("unit(monster, 21).life >= 0"), (false, Value::Null));
        assert!(check("ui.open(inventory)").0);
        assert!(!check("ui.open(stash)").0);
        assert!(check("ui.open(stash) == false").0);
        assert!(check("count(monster, *) == 2").0);
    }

    #[test]
    fn changes_compares_with_the_tick_before() {
        let c = parse_check("unit(monster, 19).anim_frame changes").unwrap();
        let now = state();
        let mut prev = state();
        assert!(!c.eval(&now, None).0, "no tick before");
        assert!(!c.eval(&now, Some(&prev)).0);
        prev["snap"]["units"][1]["fr"] = json!(256);
        assert_eq!(c.eval(&now, Some(&prev)), (true, json!(2)));
    }

    #[test]
    fn bad_checks_are_refused() {
        for bad in [
            "player.mode ==",
            "player.mode == \"cast",
            "player.pos == (1, 2",
            "player.mode == a+b",
            "nothing == 1",
        ] {
            assert!(parse_check(bad).is_err(), "{bad:?} parsed");
        }
    }
}
