//! The dev command channel: the commands the overlay's console sends to
//! the local server, their text parser, and the server's reply.
//!
//! The client console calls [`parse`] on each typed line and hands the
//! resulting [`DevCommand`] to the server, which answers with a
//! [`DevReply`]. This is a D2RUST development tool: nothing here is a game
//! message, the channel is in-process only and versioned by
//! [`DEV_VERSION`].

use std::fmt;

/// Version of the dev command channel. Bump on any change of
/// [`DevCommand`] or [`DevReply`].
pub const DEV_VERSION: u32 = 2;

/// An on/off switch command's argument: `None` toggles.
pub type Switch = Option<bool>;

/// A level, item or monster named by id / code or by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A numeric id.
    Id(u32),
    /// Text as typed (codes and names; the server resolves it).
    Name(String),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Id(n) => write!(f, "{n}"),
            Target::Name(s) => f.write_str(s),
        }
    }
}

/// Item quality (`itemquality` numbering, 1..8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// Low quality (cracked, damaged, crude).
    Low = 1,
    /// Normal quality.
    Normal = 2,
    /// Superior quality.
    Superior = 3,
    /// Magic.
    Magic = 4,
    /// Set item.
    Set = 5,
    /// Rare.
    Rare = 6,
    /// Unique.
    Unique = 7,
    /// Crafted.
    Crafted = 8,
}

impl Quality {
    /// Every quality with its console keyword.
    pub const ALL: [(&'static str, Quality); 8] = [
        ("low", Quality::Low),
        ("normal", Quality::Normal),
        ("superior", Quality::Superior),
        ("magic", Quality::Magic),
        ("set", Quality::Set),
        ("rare", Quality::Rare),
        ("unique", Quality::Unique),
        ("crafted", Quality::Crafted),
    ];

    /// The quality whose keyword is `s` (ASCII case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(s))
            .map(|&(_, q)| q)
    }

    /// The console keyword of this quality.
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, q)| *q == self)
            .map_or("?", |(n, _)| n)
    }
}

/// `kill`'s range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KillRange {
    /// Monsters near the player.
    Screen,
    /// Every monster in the level.
    Level,
}

/// Difficulty (0 normal, 1 nightmare, 2 hell).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    /// Normal.
    Normal = 0,
    /// Nightmare.
    Nightmare = 1,
    /// Hell.
    Hell = 2,
}

impl Difficulty {
    /// The console keyword of this difficulty.
    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Normal => "normal",
            Difficulty::Nightmare => "nightmare",
            Difficulty::Hell => "hell",
        }
    }
}

/// One dev command; [`COMMANDS`] lists the keywords and usage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DevCommand {
    /// `help [command]`.
    Help(Option<String>),
    /// `god`: no death; life, mana and stamina refilled every tick.
    God(Switch),
    /// `heal`: life, mana and stamina to maximum.
    Heal,
    /// `level n`: raise the character to level n.
    Level(u32),
    /// `level +n`: n levels up from the current one.
    LevelAdd(u32),
    /// `gold n`: add gold to the inventory (capped).
    Gold(u32),
    /// `statpts n`: add unspent stat points.
    StatPoints(u32),
    /// `skillpts n`: add unspent skill points.
    SkillPoints(u32),
    /// `town n`: warp to the town of act n.
    Town(u8),
    /// `warp`: warp to a level.
    Warp(Target),
    /// `waypoints`: know every waypoint.
    Waypoints,
    /// `acts`: open acts 2-5.
    Acts,
    /// `quest slot bit [on|off]`: set or clear one quest flag bit.
    Quest {
        /// Quest flag slot.
        slot: u8,
        /// Bit of the slot.
        bit: u8,
        /// Set the bit (false: clear it).
        on: bool,
    },
    /// `item`: create an item.
    Item {
        /// Item code or name.
        item: Target,
        /// Quality of the new item.
        quality: Quality,
        /// Item level, if given.
        ilvl: Option<u8>,
        /// Drop it on the ground instead of the inventory.
        ground: bool,
    },
    /// `identify`: identify every item the player holds.
    Identify,
    /// `kill`: kill monsters in the range.
    Kill(KillRange),
    /// `spawn`: spawn monsters next to the player.
    Spawn {
        /// Monster id or name.
        monster: Target,
        /// How many to spawn.
        count: u8,
    },
    /// `reveal`: reveal the automap of the level.
    Reveal,
    /// `speed percent`: faster walk/run (0: off).
    Speed(u32),
    /// `nocd`: no skill cooldown.
    NoCooldown(Switch),
    /// `freemana`: mana refilled every tick.
    FreeMana(Switch),
    /// `difficulty`: how to restart on another difficulty.
    Difficulty(Difficulty),
}

/// The switch states after a command, for the panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DevSwitchState {
    /// `god` is on.
    pub god: bool,
    /// `nocd` is on.
    pub nocd: bool,
    /// `freemana` is on.
    pub freemana: bool,
    /// Current `speed` percent (0: off).
    pub speed: u32,
}

/// The server's answer to one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevReply {
    /// The command succeeded.
    pub ok: bool,
    /// Text for the console.
    pub text: String,
    /// Switch states after the command.
    pub switches: DevSwitchState,
}

impl DevReply {
    /// A success reply with default switch states.
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            ok: true,
            text: text.into(),
            switches: DevSwitchState::default(),
        }
    }
    /// A failure reply with default switch states.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: text.into(),
            switches: DevSwitchState::default(),
        }
    }
}

/// The command table: keyword, usage, one-line description.
pub const COMMANDS: [(&str, &str, &str); 27] = [
    (
        "help",
        "help [command]",
        "list the commands, or one command's usage",
    ),
    (
        "god",
        "god [on|off]",
        "no death; life/mana/stamina refilled every tick",
    ),
    ("heal", "heal", "life, mana and stamina to maximum"),
    (
        "level",
        "level <1..99>",
        "raise the character to that level",
    ),
    ("gold", "gold <n>", "add gold to the inventory (capped)"),
    ("statpts", "statpts <n>", "add unspent stat points"),
    ("skillpts", "skillpts <n>", "add unspent skill points"),
    ("town", "town <1..5>", "warp to an act's town"),
    ("warp", "warp <level id|name>", "warp to a level"),
    (
        "waypoints",
        "waypoints",
        "know every waypoint (all difficulties)",
    ),
    (
        "acts",
        "acts",
        "open acts 2-5 (quest flags, all difficulties)",
    ),
    (
        "quest",
        "quest <slot 0..40> <bit 0..15> [on|off]",
        "set or clear one quest flag bit (all difficulties)",
    ),
    (
        "item",
        "item <code|name> [quality] [ilvl <n>] [ground]",
        "create an item (quality: low normal superior magic set rare unique crafted)",
    ),
    (
        "identify",
        "identify",
        "identify every item the player holds",
    ),
    (
        "kill",
        "kill [screen|level]",
        "kill monsters near the player or in the level",
    ),
    (
        "spawn",
        "spawn <monster id|name> [count]",
        "spawn monsters next to the player",
    ),
    ("reveal", "reveal", "reveal the automap of the level"),
    (
        "speed",
        "speed <percent 0..1000>",
        "faster walk/run (0: off)",
    ),
    ("nocd", "nocd [on|off]", "no skill cooldown"),
    ("freemana", "freemana [on|off]", "mana refilled every tick"),
    (
        "difficulty",
        "difficulty <normal|nightmare|hell>",
        "how to restart on another difficulty",
    ),
    ("clear", "clear", "clear the console (client only)"),
    (
        "animation",
        "animation [original|smooth]",
        "compare source and generated animation frames (client only)",
    ),
    ("info", "info", "toggle the info overlay (client only)"),
    (
        "bugreport",
        "bugreport [note]",
        "write a bug report folder (client only)",
    ),
    (
        "save",
        "save <name>",
        "save the character and place to a slot (client only)",
    ),
    (
        "load",
        "load [name]",
        "load a slot (restarts the game), or list the slots",
    ),
];

/// The console's own commands: [`parse`] answers `Ok(None)`, the client
/// runs them.
pub const CLIENT_COMMANDS: [&str; 6] = ["clear", "animation", "info", "bugreport", "save", "load"];

/// The help text: every command, or one command's usage.
pub fn help(topic: Option<&str>) -> Result<String, String> {
    match topic {
        None => Ok(COMMANDS
            .iter()
            .map(|(k, _, _)| *k)
            .collect::<Vec<_>>()
            .join(" ")),
        Some(t) => COMMANDS
            .iter()
            .find(|(k, _, _)| k.eq_ignore_ascii_case(t))
            .map(|(_, u, d)| format!("{u}: {d}"))
            .ok_or_else(|| format!("help: no command {t:?}")),
    }
}

/// The keywords that start with `prefix` (Tab completion).
pub fn complete(prefix: &str) -> Vec<&'static str> {
    let p = prefix.to_ascii_lowercase();
    COMMANDS
        .iter()
        .map(|(k, _, _)| *k)
        .filter(|k| k.starts_with(&p))
        .collect()
}

fn usage(kw: &str) -> String {
    let u = COMMANDS
        .iter()
        .find(|(k, _, _)| *k == kw)
        .map_or(kw, |(_, u, _)| u);
    format!("usage: {u}")
}

fn number(kw: &str, s: &str, lo: u32, hi: u32) -> Result<u32, String> {
    s.parse::<u32>()
        .ok()
        .filter(|n| (lo..=hi).contains(n))
        .ok_or_else(|| format!("{kw}: {s:?} is not a number in {lo}..={hi}"))
}

fn switch(kw: &str, args: &[&str]) -> Result<Switch, String> {
    match args {
        [] => Ok(None),
        [a] if a.eq_ignore_ascii_case("on") => Ok(Some(true)),
        [a] if a.eq_ignore_ascii_case("off") => Ok(Some(false)),
        _ => Err(usage(kw)),
    }
}

fn none(kw: &str, args: &[&str]) -> Result<(), String> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(usage(kw))
    }
}

/// A target: a number is an id, anything else the joined text.
fn target(kw: &str, args: &[&str]) -> Result<Target, String> {
    match args {
        [] => Err(usage(kw)),
        [n] if n.bytes().all(|b| b.is_ascii_digit()) => n
            .parse()
            .map(Target::Id)
            .map_err(|_| format!("{kw}: {n:?} is too big")),
        _ => Ok(Target::Name(args.join(" "))),
    }
}

/// Parses one console line. The console's own
/// commands ([`CLIENT_COMMANDS`]) are not server commands: `Ok(None)`.
pub fn parse(line: &str) -> Result<Option<DevCommand>, String> {
    let toks: Vec<&str> = line.split_whitespace().collect();
    let Some((kw, args)) = toks.split_first() else {
        return Err("empty line (try help)".into());
    };
    let kw = kw.to_ascii_lowercase();
    let kw = kw.as_str();
    Ok(Some(match kw {
        "help" => match args {
            [] => DevCommand::Help(None),
            [t] => DevCommand::Help(Some(t.to_ascii_lowercase())),
            _ => return Err(usage(kw)),
        },
        "clear" | "info" => {
            none(kw, args)?;
            return Ok(None);
        }
        "animation" => {
            match args {
                [] => {}
                [a] if a.eq_ignore_ascii_case("original")
                    || a.eq_ignore_ascii_case("smooth")
                    || a.eq_ignore_ascii_case("on")
                    || a.eq_ignore_ascii_case("off") => {}
                _ => return Err(usage(kw)),
            }
            return Ok(None);
        }
        "save" => match args {
            [_] => return Ok(None),
            _ => return Err(usage(kw)),
        },
        "load" => match args {
            [] | [_] => return Ok(None),
            _ => return Err(usage(kw)),
        },
        "bugreport" => return Ok(None),
        "god" => DevCommand::God(switch(kw, args)?),
        "nocd" => DevCommand::NoCooldown(switch(kw, args)?),
        "freemana" => DevCommand::FreeMana(switch(kw, args)?),
        "heal" => none(kw, args).map(|()| DevCommand::Heal)?,
        "waypoints" => none(kw, args).map(|()| DevCommand::Waypoints)?,
        "acts" => none(kw, args).map(|()| DevCommand::Acts)?,
        "identify" => none(kw, args).map(|()| DevCommand::Identify)?,
        "reveal" => none(kw, args).map(|()| DevCommand::Reveal)?,
        "level" | "gold" | "statpts" | "skillpts" | "town" | "speed" => {
            let [a] = args else {
                return Err(usage(kw));
            };
            match kw {
                "level" => match a.strip_prefix('+') {
                    Some(n) => DevCommand::LevelAdd(number(kw, n, 1, 98)?),
                    None => DevCommand::Level(number(kw, a, 1, 99)?),
                },
                "gold" => DevCommand::Gold(number(kw, a, 1, 10_000_000)?),
                "statpts" => DevCommand::StatPoints(number(kw, a, 1, 1000)?),
                "skillpts" => DevCommand::SkillPoints(number(kw, a, 1, 1000)?),
                "town" => DevCommand::Town(number(kw, a, 1, 5)? as u8),
                _ => DevCommand::Speed(number(kw, a, 0, 1000)?),
            }
        }
        "warp" => DevCommand::Warp(target(kw, args)?),
        "quest" => {
            let (slot, bit, on) = match args {
                [s, b] => (s, b, true),
                [s, b, o] if o.eq_ignore_ascii_case("on") => (s, b, true),
                [s, b, o] if o.eq_ignore_ascii_case("off") => (s, b, false),
                _ => return Err(usage(kw)),
            };
            DevCommand::Quest {
                slot: number(kw, slot, 0, 40)? as u8,
                bit: number(kw, bit, 0, 15)? as u8,
                on,
            }
        }
        "kill" => DevCommand::Kill(match args {
            [] => KillRange::Screen,
            [a] if a.eq_ignore_ascii_case("screen") => KillRange::Screen,
            [a] if a.eq_ignore_ascii_case("level") => KillRange::Level,
            _ => return Err(usage(kw)),
        }),
        "spawn" => {
            // A trailing number after a name (or after an id) is the count.
            let (count, rest) = match args {
                [rest @ .., c] if !rest.is_empty() && c.bytes().all(|b| b.is_ascii_digit()) => {
                    (number(kw, c, 1, 20)? as u8, rest)
                }
                _ => (1, args),
            };
            DevCommand::Spawn {
                monster: target(kw, rest)?,
                count,
            }
        }
        "item" => parse_item(args)?,
        "difficulty" => {
            let [a] = args else {
                return Err(usage(kw));
            };
            DevCommand::Difficulty(match a.to_ascii_lowercase().as_str() {
                "normal" | "0" => Difficulty::Normal,
                "nightmare" | "nm" | "1" => Difficulty::Nightmare,
                "hell" | "2" => Difficulty::Hell,
                _ => return Err(usage(kw)),
            })
        }
        _ => return Err(format!("unknown command {kw:?} (try help)")),
    }))
}

/// `item <code|name> [quality] [ilvl <n>] [ground]`: the options are read
/// from the end; what is left is the item.
fn parse_item(args: &[&str]) -> Result<DevCommand, String> {
    let mut rest = args;
    let mut quality = None;
    let mut ilvl = None;
    let mut ground = false;
    loop {
        match rest {
            [r @ .., g] if g.eq_ignore_ascii_case("ground") && !ground => {
                ground = true;
                rest = r;
            }
            [r @ .., k, n] if k.eq_ignore_ascii_case("ilvl") && ilvl.is_none() => {
                ilvl = Some(number("item", n, 1, 99)? as u8);
                rest = r;
            }
            [r @ .., q] if quality.is_none() && Quality::parse(q).is_some() && !r.is_empty() => {
                quality = Quality::parse(q);
                rest = r;
            }
            _ => break,
        }
    }
    Ok(DevCommand::Item {
        item: match target("item", rest)? {
            Target::Id(_) => return Err("item: give a code or a name, not a number".into()),
            t => t,
        },
        quality: quality.unwrap_or(Quality::Normal),
        ilvl,
        ground,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> DevCommand {
        parse(s).unwrap().unwrap()
    }

    /// Every command keyword parses to its command; client-only ones give
    /// `None`.
    #[test]
    fn every_command_parses() {
        assert_eq!(p("god"), DevCommand::God(None));
        assert_eq!(p("GOD on"), DevCommand::God(Some(true)));
        assert_eq!(p("nocd off"), DevCommand::NoCooldown(Some(false)));
        assert_eq!(p("freemana"), DevCommand::FreeMana(None));
        assert_eq!(p("heal"), DevCommand::Heal);
        assert_eq!(p("level 30"), DevCommand::Level(30));
        assert_eq!(p("level +10"), DevCommand::LevelAdd(10));
        assert_eq!(p("gold 5000"), DevCommand::Gold(5000));
        assert_eq!(p("statpts 5"), DevCommand::StatPoints(5));
        assert_eq!(p("skillpts 3"), DevCommand::SkillPoints(3));
        assert_eq!(p("town 2"), DevCommand::Town(2));
        assert_eq!(p("warp 35"), DevCommand::Warp(Target::Id(35)));
        assert_eq!(
            p("warp  Catacombs   Level 2"),
            DevCommand::Warp(Target::Name("Catacombs Level 2".into()))
        );
        assert_eq!(p("waypoints"), DevCommand::Waypoints);
        assert_eq!(p("acts"), DevCommand::Acts);
        assert_eq!(p("identify"), DevCommand::Identify);
        assert_eq!(
            p("quest 37 8"),
            DevCommand::Quest {
                slot: 37,
                bit: 8,
                on: true
            }
        );
        assert_eq!(
            p("QUEST 37 7 off"),
            DevCommand::Quest {
                slot: 37,
                bit: 7,
                on: false
            }
        );
        assert_eq!(p("kill"), DevCommand::Kill(KillRange::Screen));
        assert_eq!(p("kill level"), DevCommand::Kill(KillRange::Level));
        assert_eq!(p("reveal"), DevCommand::Reveal);
        assert_eq!(p("speed 0"), DevCommand::Speed(0));
        assert_eq!(
            p("difficulty nm"),
            DevCommand::Difficulty(Difficulty::Nightmare)
        );
        assert_eq!(p("help"), DevCommand::Help(None));
        assert_eq!(p("help Item"), DevCommand::Help(Some("item".into())));
        assert_eq!(parse("clear"), Ok(None));
        for l in [
            "info",
            "bugreport",
            "bugreport stuck here",
            "save a1",
            "load",
            "load a1",
            "animation",
            "animation original",
            "animation smooth",
            "animation on",
            "animation off",
        ] {
            assert_eq!(parse(l), Ok(None), "{l}");
        }
        assert!(parse("save").is_err() && parse("info x").is_err());
    }

    /// `item` and `spawn` options are taken from the end of the line.
    #[test]
    fn item_and_spawn_options_are_read_from_the_end() {
        assert_eq!(
            p("item hax magic ilvl 30 ground"),
            DevCommand::Item {
                item: Target::Name("hax".into()),
                quality: Quality::Magic,
                ilvl: Some(30),
                ground: true,
            }
        );
        assert_eq!(
            p("item Short Sword"),
            DevCommand::Item {
                item: Target::Name("Short Sword".into()),
                quality: Quality::Normal,
                ilvl: None,
                ground: false,
            }
        );
        assert_eq!(
            p("item ring unique"),
            DevCommand::Item {
                item: Target::Name("ring".into()),
                quality: Quality::Unique,
                ilvl: None,
                ground: false,
            }
        );
        // A quality word alone is the item's name, not an option.
        assert_eq!(
            p("item rare"),
            DevCommand::Item {
                item: Target::Name("rare".into()),
                quality: Quality::Normal,
                ilvl: None,
                ground: false,
            }
        );
        assert_eq!(
            p("spawn fallen 5"),
            DevCommand::Spawn {
                monster: Target::Name("fallen".into()),
                count: 5,
            }
        );
        assert_eq!(
            p("spawn 19"),
            DevCommand::Spawn {
                monster: Target::Id(19),
                count: 1,
            }
        );
        assert_eq!(
            p("spawn 19 3"),
            DevCommand::Spawn {
                monster: Target::Id(19),
                count: 3,
            }
        );
    }

    /// Bad lines give an error that names the problem.
    #[test]
    fn bad_lines_name_the_problem() {
        for (line, needle) in [
            ("", "empty"),
            ("fly", "unknown command"),
            ("level 0", "1..=99"),
            ("level 100", "1..=99"),
            ("level", "usage: level"),
            ("level 3 4", "usage: level"),
            ("town 6", "1..=5"),
            ("god maybe", "usage: god"),
            ("heal now", "usage: heal"),
            ("kill all", "usage: kill"),
            ("item", "usage: item"),
            ("item 12", "not a number"),
            ("item hax ilvl 0", "1..=99"),
            ("spawn fallen 21", "1..=20"),
            ("speed 2000", "0..=1000"),
            ("difficulty easy", "usage: difficulty"),
            ("warp", "usage: warp"),
            ("quest", "usage: quest"),
            ("quest 41 0", "0..=40"),
            ("quest 3 16", "0..=15"),
            ("quest 3 1 maybe", "usage: quest"),
            ("animation fast", "usage: animation"),
            ("animation on now", "usage: animation"),
        ] {
            let e = parse(line).expect_err(line);
            assert!(e.contains(needle), "{line:?}: {e}");
        }
    }

    /// Tab completion and `help`.
    #[test]
    fn completion_and_help() {
        assert_eq!(complete("he"), vec!["help", "heal"]);
        assert_eq!(complete("god"), vec!["god"]);
        assert!(complete("zz").is_empty());
        assert!(help(None).unwrap().contains("freemana"));
        assert_eq!(
            help(Some("town")).unwrap(),
            "town <1..5>: warp to an act's town"
        );
        assert!(help(Some("nope")).is_err());
        // Every keyword the parser takes is in the table.
        for (k, _, _) in COMMANDS {
            let r = parse(k);
            assert!(
                !matches!(&r, Err(e) if e.contains("unknown")),
                "{k} not parsed"
            );
        }
    }
}
