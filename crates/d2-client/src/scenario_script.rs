//! Scenario scripts: the strict line parser, the canonical writer and the
//! typed-message encoder (references resolved against a [`World`]).
//!
//! Two versions share one parser:
//! - `scenario 1`: the legacy form (a `game` line, every header line
//!   required, message, spawn and poke steps).
//! - `scenario 2`: what `repro` runs (`app::repro`, reference in
//!   `docs/tools/scenarios.md`). The header lines other than `name` and
//!   `end` have defaults, there is no `game` line, a character comes from
//!   `save FILE` or inline `char` lines (with `char hand`), and steps add
//!   `at T dev CMD` (a dev console line), `at T input STEP` (a step of
//!   the shared `--input` grammar) and checks: `at T check EXPR` (holds
//!   after tick T) and `by T check EXPR` (holds after some tick up to T),
//!   EXPR in the grammar of `app::repro::query`.
//!
//! A v1 file still parses and writes back as v1.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use d2_proto::schema::{ClientMessage, FieldType, SizeRule};
use d2_proto::CLIENT_MESSAGES;

/// The first script format version (`scenario 1`).
pub const SCENARIO_FORMAT_VERSION: u32 = 1;

/// The `repro` script format version (`scenario 2`).
pub const SCENARIO_FORMAT_V2: u32 = 2;

/// The seed a v2 script without a `seed` line runs with.
pub const V2_DEFAULT_SEED: u32 = 1;

/// Largest `end` tick (§2 header table).
pub const MAX_END: u32 = 1_000_000;

/// Largest raw message (`sim/intents-events.md` §2.1: 0x204 bytes).
pub const MAX_HEX: usize = 0x204;

/// A script line that does not parse (§2 rule 3).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct ScriptError {
    /// 1-based line number (0: the script as a whole).
    pub line: usize,
    pub message: String,
}

/// Game difficulty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Normal,
    Nightmare,
    Hell,
}

impl Difficulty {
    pub fn index(self) -> u8 {
        self as u8
    }
    fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Nightmare => "nightmare",
            Self::Hell => "hell",
        }
    }
}

/// A stream of `record` (§2 recording table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stream {
    S2c,
    Rng,
    Units,
    Stats,
    Frames,
}

impl Stream {
    pub const ALL: [Stream; 5] = [Self::S2c, Self::Rng, Self::Units, Self::Stats, Self::Frames];
    pub fn name(self) -> &'static str {
        match self {
            Self::S2c => "s2c",
            Self::Rng => "rng",
            Self::Units => "units",
            Self::Stats => "stats",
            Self::Frames => "frames",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.name() == s)
    }
}

/// Where an inline item goes (§2 character table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Inv(u8, u8),
    Stash(u8, u8),
    Cube(u8, u8),
    Belt(u8),
    Body(u8),
    /// In socket n of the nearest earlier item not itself in a socket.
    Socket(u8),
}

/// Item quality (`items/quality.md`: 1 low … 8 crafted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Low = 1,
    Normal,
    Superior,
    Magic,
    Set,
    Rare,
    Unique,
    Crafted,
}

impl Quality {
    pub const ALL: [Quality; 8] = [
        Self::Low,
        Self::Normal,
        Self::Superior,
        Self::Magic,
        Self::Set,
        Self::Rare,
        Self::Unique,
        Self::Crafted,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::Superior => "superior",
            Self::Magic => "magic",
            Self::Set => "set",
            Self::Rare => "rare",
            Self::Unique => "unique",
            Self::Crafted => "crafted",
        }
    }
}

/// An inline item (§2 item table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub code: String,
    pub place: Place,
    pub quality: Option<Quality>,
    pub ilvl: Option<u8>,
    /// Magic prefix rows (≤ 3).
    pub prefixes: Vec<u16>,
    /// Magic suffix rows (≤ 3).
    pub suffixes: Vec<u16>,
    pub unique: Option<u16>,
    pub set: Option<u16>,
    pub runeword: Option<u16>,
    pub sockets: Option<u8>,
}

impl Item {
    /// A plain item at `place`.
    pub fn plain(code: &str, place: Place) -> Self {
        Self {
            code: code.to_owned(),
            place,
            quality: None,
            ilvl: None,
            prefixes: Vec::new(),
            suffixes: Vec::new(),
            unique: None,
            set: None,
            runeword: None,
            sockets: None,
        }
    }
}

/// What a `spawn` step creates (§3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnKind {
    Normal,
    RandomBoss,
    Champion,
    Unique,
}

impl SpawnKind {
    pub const ALL: [SpawnKind; 4] = [Self::Normal, Self::RandomBoss, Self::Champion, Self::Unique];
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::RandomBoss => "random-boss",
            Self::Champion => "champion",
            Self::Unique => "unique",
        }
    }
}

/// A `spawn` step (§3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawn {
    /// monstats row.
    pub class: u32,
    pub x: Arg,
    pub y: Arg,
    pub kind: SpawnKind,
    /// Search a free point near (x, y) instead of requiring it.
    pub free: bool,
    /// monumod rows: one for `champion`, 1–9 for `unique`, none else.
    pub umods: Vec<u8>,
}

/// `char at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Default,
    At(u32, u32),
}

/// An inline character (§2 character table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inline {
    pub class: u8,
    pub level: u8,
    pub act: u8,
    pub area: u32,
    pub at: Start,
    /// (itemstatcost row, base value), script order, ids unique.
    pub stats: Vec<(u16, i32)>,
    /// (skill id, hard points), script order, ids unique.
    pub skills: Vec<(u16, u8)>,
    /// Level ids, script order, unique.
    pub waypoints: Vec<u32>,
    /// (quest index, quest flags) on the game's difficulty, unique.
    pub quests: Vec<(u8, u16)>,
    pub items: Vec<Item>,
    /// The save's map ID (`.d2s` +0xAB), when the character has one
    /// for this difficulty: the DRLG seed in place of `init` (§4 rule 1).
    pub map: Option<u32>,
    /// v2 `char hand left|right SKILL`: the skill on the left and right
    /// mouse button at the start.
    pub hands: [Option<u16>; 2],
}

/// A reference (§3 rule 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ref {
    Player,
    /// `@x±N`.
    X(i32),
    /// `@y±N`.
    Y(i32),
    /// `@<type>[:<class>][#<n>][@near]`.
    Unit {
        ty: u8,
        class: Option<u32>,
        n: u32,
        /// `@near`: the n-th nearest of the type (and class) to the
        /// player, instead of the n-th by GUID.
        near: bool,
    },
    /// `@wp[#<n>]`.
    Waypoint(u32),
}

impl std::fmt::Display for Ref {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let nth = |f: &mut std::fmt::Formatter<'_>, n: u32| {
            if n == 0 {
                Ok(())
            } else {
                write!(f, "#{n}")
            }
        };
        let off = |f: &mut std::fmt::Formatter<'_>, axis: &str, d: i32| match d {
            0 => write!(f, "@{axis}"),
            d if d > 0 => write!(f, "@{axis}+{d}"),
            d => write!(f, "@{axis}-{}", d.unsigned_abs()),
        };
        match *self {
            Self::Player => write!(f, "@player"),
            Self::X(d) => off(f, "x", d),
            Self::Y(d) => off(f, "y", d),
            Self::Unit { ty, class, n, near } => {
                write!(f, "@{ty}")?;
                if let Some(c) = class {
                    write!(f, ":{c}")?;
                }
                nth(f, n)?;
                if near {
                    write!(f, "@near")?;
                }
                Ok(())
            }
            Self::Waypoint(n) => {
                write!(f, "@wp")?;
                nth(f, n)
            }
        }
    }
}

/// A field value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    Num(u32),
    Ref(Ref),
}

/// A step's message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepMsg {
    /// Read-only save/load comparison at a runner checkpoint (scenario 2).
    Checkpoint,
    /// Send a player key only while a read-only state condition holds.
    ConditionalKey {
        key: u32,
        expr: String,
    },
    Hex(Vec<u8>),
    /// A typed message: the client-messages row and its fields in layout
    /// order.
    Typed {
        id: u8,
        fields: Vec<(String, Arg)>,
    },
    /// Not a message: a monster spawned by the server (§3.1).
    Spawn(Spawn),
    /// Not a message: a state change made directly (`tools/poke.md` §3).
    Poke(d2_sim::poke::Directive),
    /// v2, not a message: a dev console line, run as the dev panel runs
    /// it (checked against the console's grammar when parsed).
    Dev(String),
    /// v2, not a message: one step of the `--input` grammar
    /// (`world_view::input_script`), as written.
    Input(String),
    /// v2, not a message: a check (`app::repro::query`), as written;
    /// `by` (written `by T check`): it must hold after some tick up to
    /// the step's tick, else (`at T check`) after that tick.
    Check {
        by: bool,
        expr: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub tick: u32,
    pub msg: StepMsg,
}

/// A parsed scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenario {
    /// The format version: [`SCENARIO_FORMAT_VERSION`] or
    /// [`SCENARIO_FORMAT_V2`].
    pub version: u32,
    /// Units for scenario times; absent in legacy files means 25 Hz.
    pub tick_rate: u32,
    pub name: String,
    /// Game creation seed, paired with the fixed secondary seed 666.
    pub seed: u32,
    /// Secondary deterministic initialization value for legacy scenarios.
    pub init: u32,
    pub difficulty: Difficulty,
    pub expansion: bool,
    pub end: u32,
    /// `variant <name>`: the test variant install both sides run on
    /// (`tools/test-variants.md` §4 rule 3); `None`: the base install.
    pub variant: Option<String>,
    /// `char save <name>`: a named legacy character save.
    pub save: Option<String>,
    /// v2 `save <file>`: a `.d2s` file the run loads (a path relative to
    /// the script's folder, or absolute).
    pub save_file: Option<String>,
    /// The inline character (what D2RUST builds until a save loader exists).
    pub character: Option<Inline>,
    /// Sorted, unique.
    pub record: Vec<Stream>,
    pub snapshot_every: Option<u32>,
    /// Sorted, unique.
    pub snapshot_at: Vec<u32>,
    /// Generic checks explicitly disabled by a v2 scenario.
    pub allow: BTreeSet<String>,
    pub steps: Vec<Step>,
}

impl Scenario {
    pub fn records(&self, s: Stream) -> bool {
        self.record.contains(&s)
    }

    /// The snapshot ticks (§2: `every` multiples, `at` ticks and `end`);
    /// empty when neither `units` nor `stats` is recorded.
    pub fn snapshot_ticks(&self) -> BTreeSet<u32> {
        let mut out = BTreeSet::new();
        if !(self.records(Stream::Units) || self.records(Stream::Stats)) {
            return out;
        }
        if let Some(n) = self.snapshot_every {
            out.extend((0..=self.end).step_by(n as usize));
        }
        out.extend(self.snapshot_at.iter().copied());
        out.insert(self.end);
        out
    }

    /// Canonical text (§2 rule 6).
    pub fn to_text(&self) -> String {
        let mut o = String::new();
        let mut line = |s: String| {
            o.push_str(&s);
            o.push('\n');
        };
        line(format!("scenario {}", self.version));
        if self.version == SCENARIO_FORMAT_V2 {
            line(format!("tick_rate {}", self.tick_rate));
        }
        line(format!("name {}", self.name));
        if self.version == SCENARIO_FORMAT_VERSION {
            line("game 1.14d".into());
        }
        line(format!("seed 0x{:08x}", self.seed));
        line(format!("init 0x{:08x}", self.init));
        line(format!("difficulty {}", self.difficulty.name()));
        line(format!(
            "expansion {}",
            if self.expansion { "yes" } else { "no" }
        ));
        line(format!("end {}", self.end));
        if let Some(v) = &self.variant {
            line(format!("variant {v}"));
        }
        if let Some(n) = &self.save {
            line(format!("char save {n}"));
        }
        if let Some(f) = &self.save_file {
            line(format!("save {f}"));
        }
        match &self.character {
            None => {}
            Some(c) => {
                line(format!("char class {}", c.class));
                line(format!("char level {}", c.level));
                line(format!("char area {} {}", c.act, c.area));
                if let Some(m) = c.map {
                    line(format!("char map 0x{m:08x}"));
                }
                line(match c.at {
                    Start::Default => "char at default".into(),
                    Start::At(x, y) => format!("char at {x} {y}"),
                });
                for (hand, skill) in ["left", "right"].iter().zip(c.hands) {
                    if let Some(id) = skill {
                        line(format!("char hand {hand} {id}"));
                    }
                }
                for (s, v) in &c.stats {
                    line(format!("char stat {s} {v}"));
                }
                for (s, l) in &c.skills {
                    line(format!("char skill {s} {l}"));
                }
                for w in &c.waypoints {
                    line(format!("char waypoint {w}"));
                }
                for (q, f) in &c.quests {
                    line(format!("char quest {q} {f}"));
                }
                for i in &c.items {
                    let mut l = format!("char item {} ", i.code);
                    match i.place {
                        Place::Inv(x, y) => l += &format!("inv {x} {y}"),
                        Place::Stash(x, y) => l += &format!("stash {x} {y}"),
                        Place::Cube(x, y) => l += &format!("cube {x} {y}"),
                        Place::Belt(s) => l += &format!("belt {s}"),
                        Place::Body(b) => l += &format!("body {b}"),
                        Place::Socket(n) => l += &format!("socket {n}"),
                    }
                    if let Some(q) = i.quality {
                        let _ = write!(l, " quality {}", q.name());
                    }
                    if let Some(v) = i.ilvl {
                        let _ = write!(l, " ilvl {v}");
                    }
                    for v in &i.prefixes {
                        let _ = write!(l, " prefix {v}");
                    }
                    for v in &i.suffixes {
                        let _ = write!(l, " suffix {v}");
                    }
                    for (k, v) in [
                        ("unique", i.unique),
                        ("set", i.set),
                        ("runeword", i.runeword),
                    ] {
                        if let Some(v) = v {
                            let _ = write!(l, " {k} {v}");
                        }
                    }
                    if let Some(v) = i.sockets {
                        let _ = write!(l, " sockets {v}");
                    }
                    line(l);
                }
            }
        }
        if !self.record.is_empty() {
            let names: Vec<&str> = self.record.iter().map(|s| s.name()).collect();
            line(format!("record {}", names.join(" ")));
        }
        if let Some(n) = self.snapshot_every {
            line(format!("snapshot every {n}"));
        }
        if !self.snapshot_at.is_empty() {
            let t: Vec<String> = self.snapshot_at.iter().map(u32::to_string).collect();
            line(format!("snapshot at {}", t.join(" ")));
        }
        for rule in &self.allow {
            line(format!("allow {rule}"));
        }
        for s in &self.steps {
            let kw = match s.msg {
                StepMsg::Check { by: true, .. } => "by",
                _ => "at",
            };
            let mut l = format!("{kw} {} ", s.tick);
            match &s.msg {
                StepMsg::Checkpoint => l.push_str("checkpoint"),
                StepMsg::ConditionalKey { key, expr } => {
                    l.push_str(&format!("key-if {key} {expr}"))
                }
                StepMsg::Hex(b) => {
                    l.push_str("hex");
                    for x in b {
                        let _ = write!(l, " {x:02x}");
                    }
                }
                StepMsg::Spawn(sp) => {
                    let arg = |a: &Arg| match a {
                        Arg::Num(x) => x.to_string(),
                        Arg::Ref(r) => r.to_string(),
                    };
                    let _ = write!(
                        l,
                        "spawn {} {} {} {}",
                        sp.class,
                        arg(&sp.x),
                        arg(&sp.y),
                        sp.kind.name()
                    );
                    if sp.free {
                        l.push_str(" free");
                    }
                    if !sp.umods.is_empty() {
                        l.push_str(" umod");
                        for u in &sp.umods {
                            let _ = write!(l, " {u}");
                        }
                    }
                }
                StepMsg::Poke(d) => {
                    let _ = write!(l, "poke {d}");
                }
                StepMsg::Dev(c) => {
                    let _ = write!(l, "dev {c}");
                }
                StepMsg::Input(i) => {
                    let _ = write!(l, "input {i}");
                }
                StepMsg::Check { expr, .. } => {
                    let _ = write!(l, "check {expr}");
                }
                StepMsg::Typed { id, fields } => {
                    let _ = write!(l, "msg {}", message(*id).name);
                    for (n, v) in fields {
                        match v {
                            Arg::Num(x) => {
                                let _ = write!(l, " {n}={x}");
                            }
                            Arg::Ref(r) => {
                                let _ = write!(l, " {n}={r}");
                            }
                        }
                    }
                }
            }
            line(l);
        }
        o
    }

    /// SHA-256 (hex) of the canonical text (FORMAT.md header
    /// `scenario_sha256`).
    pub fn sha256(&self) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(self.to_text().as_bytes())
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            })
    }

    /// Parses a script (§2, §3).
    pub fn parse(text: &str) -> Result<Self, ScriptError> {
        Parser::default().run(text)
    }
}

fn message(id: u8) -> &'static ClientMessage {
    &CLIENT_MESSAGES[usize::from(id)]
}

/// A typed message's size and fields, or why it cannot be typed (§3
/// rule 1).
fn typed_layout(m: &ClientMessage) -> Result<usize, String> {
    let SizeRule::Fixed(size) = m.transport_size else {
        return Err(format!("{} has no fixed size: write it as hex", m.name));
    };
    if size == 0 {
        return Err(format!("{} is never a valid message", m.name));
    }
    for f in m.layout {
        match f.ty {
            FieldType::U8
            | FieldType::U16
            | FieldType::U32
            | FieldType::Bits(_)
            | FieldType::Bit(_)
                if f.offset.is_some() => {}
            _ => {
                return Err(format!(
                    "{} field {} cannot be typed: write the message as hex",
                    m.name, f.name
                ))
            }
        }
    }
    Ok(usize::from(size))
}

/// Largest value a field holds.
fn field_max(ty: FieldType) -> u32 {
    match ty {
        FieldType::U8 => 0xFF,
        FieldType::U16 => 0xFFFF,
        FieldType::Bits(n) if n < 32 => (1u32 << n) - 1,
        FieldType::Bit(_) => 1,
        _ => u32::MAX,
    }
}

fn find_message(name: &str) -> Option<&'static ClientMessage> {
    CLIENT_MESSAGES
        .iter()
        .find(|m| m.name == name && m.name != "-")
}

/// The game state references resolve against (§3 rules 3–4).
pub trait World {
    /// The character's GUID and position (sub-tiles).
    fn player(&self) -> Option<(u32, i32, i32)>;
    /// Every unit of the unit lists.
    fn units(&self) -> Vec<UnitRef>;
}

/// A unit as references see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitRef {
    pub ty: u8,
    pub class: u32,
    pub guid: u32,
    /// The unit's sub-tile position, when it has one.
    pub pos: Option<(i32, i32)>,
    /// An object whose `objects` row has operate function 23.
    pub waypoint: bool,
}

/// Resolves one reference; `Err` names why it did not resolve.
pub fn resolve(r: &Ref, w: &dyn World) -> Result<i64, String> {
    let nth = |mut guids: Vec<u32>, n: u32| {
        guids.sort_unstable();
        guids
            .get(n as usize)
            .map(|&g| i64::from(g))
            .ok_or_else(|| format!("{r}: no such unit"))
    };
    let nearest = |mut units: Vec<(u32, Option<(i32, i32)>)>, n: u32| {
        let player = w.player().map(|(_, x, y)| (x, y));
        let dist = |pos: Option<(i32, i32)>| match (player, pos) {
            (Some((px, py)), Some((x, y))) => {
                let (dx, dy) = (i64::from(x - px), i64::from(y - py));
                dx * dx + dy * dy
            }
            _ => i64::MAX,
        };
        units.sort_unstable_by_key(|&(guid, pos)| (dist(pos), guid));
        units
            .get(n as usize)
            .map(|&(g, _)| i64::from(g))
            .ok_or_else(|| format!("{r}: no such unit"))
    };
    match *r {
        Ref::Player => w
            .player()
            .map(|p| i64::from(p.0))
            .ok_or_else(|| format!("{r}: no player")),
        Ref::X(d) | Ref::Y(d) => {
            let (_, x, y) = w.player().ok_or_else(|| format!("{r}: no player"))?;
            let base = if matches!(r, Ref::X(_)) { x } else { y };
            Ok(i64::from(base) + i64::from(d))
        }
        Ref::Unit { ty, class, n, near } => {
            let units: Vec<UnitRef> = w
                .units()
                .into_iter()
                .filter(|u| u.ty == ty && class.is_none_or(|c| u.class == c))
                .collect();
            if near {
                nearest(units.into_iter().map(|u| (u.guid, u.pos)).collect(), n)
            } else {
                nth(units.into_iter().map(|u| u.guid).collect(), n)
            }
        }
        Ref::Waypoint(n) => nth(
            w.units()
                .into_iter()
                .filter(|u| u.ty == 2 && u.waypoint)
                .map(|u| u.guid)
                .collect(),
            n,
        ),
    }
}

/// A spawn step's position (§3.1 rule 2), or the reference that did not
/// resolve.
pub fn spawn_position(sp: &Spawn, w: &dyn World) -> Result<(i32, i32), Unresolved> {
    let one = |a: &Arg| match a {
        Arg::Num(v) => i32::try_from(*v).map_err(|_| Unresolved {
            reference: v.to_string(),
            why: format!("{v} is not a position"),
        }),
        Arg::Ref(r) => resolve(r, w)
            .and_then(|v| i32::try_from(v).map_err(|_| format!("{r}: {v} is not a position")))
            .map_err(|why| Unresolved {
                reference: r.to_string(),
                why,
            }),
    };
    Ok((one(&sp.x)?, one(&sp.y)?))
}

/// A step that did not resolve (§3 rule 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unresolved {
    /// The reference as written (the trace's `unresolved`).
    pub reference: String,
    /// Why, for diagnostics.
    pub why: String,
}

/// The bytes of a step (§3 rule 2), or the reference that did not
/// resolve (§3 rule 5).
pub fn encode(msg: &StepMsg, w: &dyn World) -> Result<Vec<u8>, Unresolved> {
    let (id, fields) = match msg {
        StepMsg::Hex(b) => return Ok(b.clone()),
        StepMsg::Typed { id, fields } => (*id, fields),
        StepMsg::Spawn(_) => {
            return Err(Unresolved {
                reference: "spawn".into(),
                why: "a spawn step is not a message".into(),
            })
        }
        StepMsg::Poke(_) => {
            return Err(Unresolved {
                reference: "poke".into(),
                why: "a poke step is not a message".into(),
            })
        }
        StepMsg::Checkpoint
        | StepMsg::ConditionalKey { .. }
        | StepMsg::Dev(_)
        | StepMsg::Input(_)
        | StepMsg::Check { .. } => {
            return Err(Unresolved {
                reference: "step".into(),
                why: "a dev, input, check or checkpoint step is not a message".into(),
            })
        }
    };
    let m = message(id);
    // Checked at parse time.
    let size = typed_layout(m).expect("typed messages are checked at parse time");
    let mut out = vec![0u8; size];
    out[0] = id;
    for f in m.layout {
        let (_, arg) = fields
            .iter()
            .find(|(n, _)| n == f.name)
            .expect("every field is checked at parse time");
        let v = match arg {
            Arg::Num(v) => *v,
            Arg::Ref(r) => {
                let unresolved = |why: String| Unresolved {
                    reference: r.to_string(),
                    why,
                };
                let v = resolve(r, w).map_err(unresolved)?;
                u32::try_from(v)
                    .ok()
                    .filter(|&v| v <= field_max(f.ty))
                    .ok_or_else(|| unresolved(format!("value {v} does not fit field {}", f.name)))?
            }
        };
        let off = usize::from(f.offset.unwrap_or(0));
        match f.ty {
            FieldType::U8 => out[off] = v as u8,
            FieldType::U16 => out[off..off + 2].copy_from_slice(&(v as u16).to_le_bytes()),
            FieldType::U32 => out[off..off + 4].copy_from_slice(&v.to_le_bytes()),
            FieldType::Bits(_) | FieldType::Bit(_) => {
                let shift = match f.ty {
                    FieldType::Bit(n) => u32::from(n),
                    _ => 0,
                };
                let old = u32::from_le_bytes(out[off..off + 4].try_into().expect("4 bytes"));
                out[off..off + 4].copy_from_slice(&(old | (v << shift)).to_le_bytes());
            }
            _ => unreachable!("typed_layout admits integer fields only"),
        }
    }
    Ok(out)
}

#[derive(Default)]
struct Parser {
    /// The `scenario N` line's version (1 or 2).
    version: u32,
    tick_rate: Option<u32>,
    save_file: Option<String>,
    hands: [Option<u16>; 2],
    name: Option<String>,
    game: bool,
    seed: Option<u32>,
    init: Option<u32>,
    char_map: Option<u32>,
    difficulty: Option<Difficulty>,
    expansion: Option<bool>,
    end: Option<u32>,
    variant: Option<String>,
    save: Option<String>,
    class: Option<u8>,
    level: Option<u8>,
    area: Option<(u8, u32)>,
    at: Option<Start>,
    stats: Vec<(u16, i32)>,
    skills: Vec<(u16, u8)>,
    waypoints: Vec<u32>,
    quests: Vec<(u8, u16)>,
    items: Vec<Item>,
    inline_lines: Option<usize>,
    record: Option<Vec<Stream>>,
    every: Option<u32>,
    at_ticks: Option<Vec<u32>>,
    allow: BTreeSet<String>,
    steps: Vec<(usize, Step)>,
}

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

fn ranged<T: TryFrom<u32>>(t: &str, lo: u32, hi: u32, what: &str) -> Result<T, String> {
    let v = num(t)?;
    if v < lo || v > hi {
        return Err(format!("{what} {v} outside {lo}..={hi}"));
    }
    T::try_from(v).map_err(|_| format!("{what} {v} out of range"))
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

fn parse_ref(t: &str) -> Result<Ref, String> {
    let body = t.strip_prefix('@').ok_or("not a reference")?;
    if body == "player" {
        return Ok(Ref::Player);
    }
    for (axis, mk) in [("x", Ref::X as fn(i32) -> Ref), ("y", Ref::Y)] {
        if let Some(rest) = body.strip_prefix(axis) {
            let d = match rest.as_bytes().first() {
                None => 0,
                Some(b'+') if !rest[1..].starts_with('-') => signed(&rest[1..])?,
                Some(b'-') if rest.len() > 1 => signed(rest)?,
                _ => return Err(format!("bad reference {t:?}")),
            };
            // `@x+0` / `@x-0` are written `@x`.
            if d == 0 && !rest.is_empty() {
                return Err(format!("{t:?}: write a zero offset as @{axis}"));
            }
            return Ok(mk(d));
        }
    }
    let (body, near) = match body.strip_suffix("@near") {
        Some(b) => (b, true),
        None => (body, false),
    };
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
        if near {
            return Err(format!("{t:?}: @near needs a unit type"));
        }
        return Ok(Ref::Waypoint(n));
    }
    let (ty, class) = match head.split_once(':') {
        Some((ty, c)) => (ty, Some(num(c)?)),
        None => (head, None),
    };
    let ty = ranged::<u8>(ty, 0, 5, "unit type")?;
    Ok(Ref::Unit { ty, class, n, near })
}

impl Parser {
    fn run(mut self, text: &str) -> Result<Scenario, ScriptError> {
        let mut version_seen = false;
        for (k, raw) in text.lines().enumerate() {
            let line = k + 1;
            // A comment starts at a token that begins with `#` (§2 rule 1).
            let toks: Vec<&str> = raw
                .split_ascii_whitespace()
                .take_while(|t| !t.starts_with('#'))
                .collect();
            if toks.is_empty() {
                continue;
            }
            let err = |message: String| ScriptError { line, message };
            if !version_seen {
                self.version = match toks.as_slice() {
                    ["scenario", "1"] => SCENARIO_FORMAT_VERSION,
                    ["scenario", "2"] => SCENARIO_FORMAT_V2,
                    ["scenario", v] => {
                        return Err(err(format!("unknown scenario format version {v}")))
                    }
                    _ => {
                        return Err(err(
                            "the first line must be `scenario 1` or `scenario 2`".into()
                        ))
                    }
                };
                version_seen = true;
                continue;
            }
            self.line(line, &toks).map_err(err)?;
        }
        if !version_seen {
            return Err(ScriptError {
                line: 0,
                message: "empty script: no `scenario` line".into(),
            });
        }
        self.finish()
    }

    fn line(&mut self, line: usize, toks: &[&str]) -> Result<(), String> {
        let rest = &toks[1..];
        let want = |n: usize| {
            if rest.len() == n {
                Ok(())
            } else {
                Err(format!(
                    "`{}` takes {n} value(s), got {}",
                    toks[0],
                    rest.len()
                ))
            }
        };
        if toks[0] != "at" && toks[0] != "by" && !self.steps.is_empty() {
            return Err(format!("`{}` after the first `at` line", toks[0]));
        }
        fn once<T>(slot: &mut Option<T>, v: T, what: &str) -> Result<(), String> {
            if slot.is_some() {
                return Err(format!("`{what}` given twice"));
            }
            *slot = Some(v);
            Ok(())
        }
        match toks[0] {
            "allow" if self.version == SCENARIO_FORMAT_V2 => {
                want(1)?;
                if !crate::app::checks::RULES.contains(&rest[0]) {
                    return Err(format!("unknown check rule {:?}", rest[0]));
                }
                self.allow.insert(rest[0].to_owned());
                Ok(())
            }
            "name" => {
                want(1)?;
                let n = rest[0];
                if !n
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                {
                    return Err(format!("name {n:?} is not [a-z0-9-]+"));
                }
                once(&mut self.name, n.to_owned(), "name")
            }
            "game" if self.version == SCENARIO_FORMAT_V2 => {
                Err("`game` is a `scenario 1` line; a `scenario 2` script has none".into())
            }
            "save" if self.version == SCENARIO_FORMAT_V2 => {
                want(1)?;
                once(&mut self.save_file, rest[0].to_owned(), "save")
            }
            "game" => {
                want(1)?;
                if rest[0] != "1.14d" {
                    return Err(format!(
                        "game {:?}: only the legacy scenario game tag is accepted",
                        rest[0]
                    ));
                }
                if self.game {
                    return Err("`game` given twice".into());
                }
                self.game = true;
                Ok(())
            }
            "seed" => {
                want(1)?;
                once(
                    &mut self.seed,
                    ranged(rest[0], 0, 0x7FFF_FFFF, "seed")?,
                    "seed",
                )
            }
            "init" => {
                want(1)?;
                once(&mut self.init, num(rest[0])?, "init")
            }
            "difficulty" => {
                want(1)?;
                let d = match rest[0] {
                    "normal" => Difficulty::Normal,
                    "nightmare" => Difficulty::Nightmare,
                    "hell" => Difficulty::Hell,
                    d => return Err(format!("unknown difficulty {d:?}")),
                };
                once(&mut self.difficulty, d, "difficulty")
            }
            "expansion" => {
                want(1)?;
                let e = match rest[0] {
                    "yes" => true,
                    "no" => false,
                    e => return Err(format!("expansion {e:?}: yes or no")),
                };
                once(&mut self.expansion, e, "expansion")
            }
            "tick_rate" => {
                if self.version != SCENARIO_FORMAT_V2 {
                    return Err("tick_rate requires scenario 2".into());
                }
                want(1)?;
                once(
                    &mut self.tick_rate,
                    ranged(rest[0], 1, 1000, "tick_rate")?,
                    "tick_rate",
                )
            }
            "end" => {
                want(1)?;
                once(&mut self.end, ranged(rest[0], 0, MAX_END, "end")?, "end")
            }
            "variant" => {
                want(1)?;
                let n = rest[0];
                if n.is_empty()
                    || !n
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                {
                    return Err(format!("variant {n:?} is not [a-z0-9-]+"));
                }
                once(&mut self.variant, n.to_owned(), "variant")
            }
            "char" => self.char_line(line, rest),
            "record" => {
                if rest.is_empty() {
                    return Err("`record` needs at least one stream".into());
                }
                let mut v = Vec::new();
                for s in rest {
                    let s = Stream::parse(s).ok_or_else(|| {
                        if *s == "c2s" {
                            "c2s is always recorded: do not list it".to_owned()
                        } else {
                            format!("unknown stream {s:?}")
                        }
                    })?;
                    if v.contains(&s) {
                        return Err(format!("stream {} listed twice", s.name()));
                    }
                    v.push(s);
                }
                v.sort();
                once(&mut self.record, v, "record")
            }
            "snapshot" => match rest {
                ["every", n] => once(
                    &mut self.every,
                    ranged(n, 1, MAX_END, "snapshot interval")?,
                    "snapshot every",
                ),
                ["at", ticks @ ..] if !ticks.is_empty() => {
                    let mut v = ticks
                        .iter()
                        .map(|t| num(t))
                        .collect::<Result<Vec<u32>, _>>()?;
                    let n = v.len();
                    v.sort_unstable();
                    v.dedup();
                    if v.len() != n {
                        return Err("a snapshot tick is listed twice".into());
                    }
                    once(&mut self.at_ticks, v, "snapshot at")
                }
                _ => Err("`snapshot every <n>` or `snapshot at <tick>...`".into()),
            },
            "at" | "by" => {
                let v2 = self.version == SCENARIO_FORMAT_V2;
                let by = toks[0] == "by";
                if by && !(v2 && rest.get(1) == Some(&"check")) {
                    return Err("`by <tick> check EXPR` (scenario 2)".into());
                }
                if rest.len() < 2 {
                    return Err("`at <tick> hex|msg ...`".into());
                }
                let tick = ranged(rest[0], 0, MAX_END, "tick")?;
                if let Some((_, prev)) = self.steps.last() {
                    if tick < prev.tick {
                        return Err(format!(
                            "step at tick {tick} after a step at tick {}",
                            prev.tick
                        ));
                    }
                }
                let msg = match rest[1] {
                    "hex" => hex(&rest[2..])?,
                    "msg" => typed(&rest[2..])?,
                    "spawn" => spawn(&rest[2..])?,
                    "poke" => StepMsg::Poke(d2_sim::poke::parse_directive(&rest[2..])?),
                    "dev" if v2 => dev_step(&rest[2..])?,
                    "input" if v2 => input_step(&rest[2..])?,
                    "key-if" if v2 && rest.len() >= 4 => {
                        let key = num(rest[2])?;
                        let expr = rest[3..].join(" ");
                        crate::app::repro::query::parse_check(&expr)?;
                        if crate::world_view::input_script::play_key(key).is_none() {
                            return Err("key-if needs a supported virtual key".into());
                        }
                        StepMsg::ConditionalKey { key, expr }
                    },
                    "check" if v2 => check_step(by, &rest[2..])?,
                    "checkpoint" if v2 && rest.len() == 2 => StepMsg::Checkpoint,
                    k if v2 => {
                        return Err(format!(
                            "unknown step kind {k:?}: hex, msg, spawn, poke, dev, input, key-if, check or checkpoint"
                        ))
                    }
                    k => return Err(format!("unknown step kind {k:?}: hex, msg, spawn or poke")),
                };
                self.steps.push((line, Step { tick, msg }));
                Ok(())
            }
            k => Err(format!("unknown keyword {k:?}")),
        }
    }

    fn char_line(&mut self, line: usize, rest: &[&str]) -> Result<(), String> {
        let Some((&kw, args)) = rest.split_first() else {
            return Err("`char` needs a field".into());
        };
        if kw == "save" && self.version == SCENARIO_FORMAT_V2 {
            return Err("`char save` is a `scenario 1` line; use `save <file>`".into());
        }
        if kw == "hand" && self.version == SCENARIO_FORMAT_V2 {
            self.inline_lines.get_or_insert(line);
            let [hand, skill] = args else {
                return Err("`char hand left|right <skill id>`".into());
            };
            let i = match *hand {
                "left" => 0,
                "right" => 1,
                h => return Err(format!("hand {h:?}: left or right")),
            };
            let id = ranged::<u16>(skill, 0, 0xFFFF, "skill id")?;
            return once(&mut self.hands[i], id, &format!("hand {hand}"));
        }
        if kw == "save" {
            let [name] = args else {
                return Err("`char save <name>`".into());
            };
            let ok = name.len() >= 2
                && name.len() <= 15
                && name.as_bytes()[0].is_ascii_alphabetic()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphabetic() || b == b'_' || b == b'-');
            if !ok {
                return Err(format!(
                    "save {name:?}: a character name, 2-15 letters, `_` or `-`, starting with a letter"
                ));
            }
            if self.save.is_some() {
                return Err("`char save` given twice".into());
            }
            self.save = Some((*name).to_owned());
            return Ok(());
        }
        self.inline_lines.get_or_insert(line);
        let want = |n: usize| {
            if args.len() == n {
                Ok(())
            } else {
                Err(format!(
                    "`char {kw}` takes {n} value(s), got {}",
                    args.len()
                ))
            }
        };
        fn once<T>(slot: &mut Option<T>, v: T, what: &str) -> Result<(), String> {
            if slot.is_some() {
                return Err(format!("`char {what}` given twice"));
            }
            *slot = Some(v);
            Ok(())
        }
        match kw {
            "class" => {
                want(1)?;
                once(&mut self.class, ranged(args[0], 0, 6, "class")?, "class")
            }
            "level" => {
                want(1)?;
                once(&mut self.level, ranged(args[0], 1, 99, "level")?, "level")
            }
            "map" => {
                want(1)?;
                once(&mut self.char_map, num(args[0])?, "map")
            }
            "area" => {
                want(2)?;
                let a = (ranged(args[0], 0, 4, "act")?, num(args[1])?);
                once(&mut self.area, a, "area")
            }
            "at" => {
                let s = match args {
                    ["default"] => Start::Default,
                    [x, y] => Start::At(num(x)?, num(y)?),
                    _ => return Err("`char at default` or `char at <x> <y>`".into()),
                };
                once(&mut self.at, s, "at")
            }
            "stat" => {
                want(2)?;
                let id = ranged::<u16>(args[0], 0, 0xFFFF, "stat id")?;
                if self.stats.iter().any(|s| s.0 == id) {
                    return Err(format!("stat {id} given twice"));
                }
                self.stats.push((id, signed(args[1])?));
                Ok(())
            }
            "skill" => {
                want(2)?;
                let id = ranged::<u16>(args[0], 0, 0xFFFF, "skill id")?;
                if self.skills.iter().any(|s| s.0 == id) {
                    return Err(format!("skill {id} given twice"));
                }
                self.skills
                    .push((id, ranged(args[1], 1, 255, "skill level")?));
                Ok(())
            }
            "waypoint" => {
                want(1)?;
                let l = num(args[0])?;
                if self.waypoints.contains(&l) {
                    return Err(format!("waypoint {l} given twice"));
                }
                self.waypoints.push(l);
                Ok(())
            }
            "quest" => {
                want(2)?;
                let q = ranged::<u8>(args[0], 0, 40, "quest index")?;
                if self.quests.iter().any(|e| e.0 == q) {
                    return Err(format!("quest {q} given twice"));
                }
                self.quests
                    .push((q, ranged(args[1], 0, 0xFFFF, "quest flags")?));
                Ok(())
            }
            "item" => {
                let item = item(args)?;
                if matches!(item.place, Place::Socket(_))
                    && !self
                        .items
                        .iter()
                        .any(|i| !matches!(i.place, Place::Socket(_)))
                {
                    return Err("a socketed item needs an earlier item to sit in".into());
                }
                self.items.push(item);
                Ok(())
            }
            k => Err(format!("unknown character field {k:?}")),
        }
    }

    fn finish(self) -> Result<Scenario, ScriptError> {
        let whole = |message: String| ScriptError { line: 0, message };
        let missing = |what: &str| whole(format!("missing `{what}` line"));
        let name = self.name.ok_or_else(|| missing("name"))?;
        let v2 = self.version == SCENARIO_FORMAT_V2;
        if !self.game && !v2 {
            return Err(missing("game"));
        }
        // v2: every header line but `name` and `end` has a default.
        let (seed, init, difficulty, expansion) = if v2 {
            (
                self.seed.unwrap_or(V2_DEFAULT_SEED),
                self.init.unwrap_or(0),
                self.difficulty.unwrap_or(Difficulty::Normal),
                self.expansion.unwrap_or(true),
            )
        } else {
            (
                self.seed.ok_or_else(|| missing("seed"))?,
                self.init.ok_or_else(|| missing("init"))?,
                self.difficulty.ok_or_else(|| missing("difficulty"))?,
                self.expansion.ok_or_else(|| missing("expansion"))?,
            )
        };
        let end = self.end.ok_or_else(|| missing("end"))?;
        let has_save = self.save.is_some() || self.save_file.is_some();
        if v2 && self.inline_lines.is_some() && has_save {
            return Err(whole(
                "a character from `save` or from inline `char` lines, not both".into(),
            ));
        }
        let character = match (self.inline_lines, has_save) {
            (None, false) if v2 => {
                return Err(whole(
                    "no character: `save <file>` or inline `char` lines".into(),
                ))
            }
            (None, false) => {
                return Err(whole(
                    "no character: `char save` or inline `char` lines".into(),
                ))
            }
            (None, true) => None,
            (Some(_), _) => {
                let (act, area) = match self.area {
                    Some(a) => a,
                    // v2: the first act's town.
                    None if v2 => (0, 1),
                    None => return Err(missing("char area")),
                };
                Some(Inline {
                    class: self.class.ok_or_else(|| missing("char class"))?,
                    level: self.level.unwrap_or(1),
                    act,
                    area,
                    at: self.at.unwrap_or(Start::Default),
                    stats: self.stats,
                    skills: self.skills,
                    waypoints: self.waypoints,
                    quests: self.quests,
                    items: self.items,
                    map: self.char_map,
                    hands: self.hands,
                })
            }
        };
        let record = self.record.unwrap_or_default();
        let snapshots = record.contains(&Stream::Units) || record.contains(&Stream::Stats);
        if (self.every.is_some() || self.at_ticks.is_some()) && !snapshots {
            return Err(whole(
                "`snapshot` lines need `units` or `stats` recorded".into(),
            ));
        }
        let snapshot_at = self.at_ticks.unwrap_or_default();
        if let Some(&t) = snapshot_at.iter().find(|&&t| t > end) {
            return Err(whole(format!("snapshot tick {t} after end {end}")));
        }
        let mut steps = Vec::with_capacity(self.steps.len());
        for (line, s) in self.steps {
            if s.tick > end {
                return Err(ScriptError {
                    line,
                    message: format!("step at tick {} after end {end}", s.tick),
                });
            }
            steps.push(s);
        }
        Ok(Scenario {
            version: self.version,
            tick_rate: self.tick_rate.unwrap_or(25),
            name,
            seed,
            init,
            difficulty,
            expansion,
            end,
            variant: self.variant,
            save: self.save,
            save_file: self.save_file,
            character,
            record,
            snapshot_every: self.every,
            snapshot_at,
            allow: self.allow,
            steps,
        })
    }
}

/// `at T dev CMD...` (v2): a dev console line the server end runs; the
/// console's own lines (`clear`, `info`) are not server commands.
fn dev_step(toks: &[&str]) -> Result<StepMsg, String> {
    let line = toks.join(" ");
    match d2_proto::dev::parse(&line)? {
        Some(_) => Ok(StepMsg::Dev(line)),
        None => Err(format!(
            "dev {line:?}: a console-only line, not a server command"
        )),
    }
}

/// `at T check EXPR` / `by T check EXPR` (v2): a check, its grammar
/// checked now (`app::repro::query`).
fn check_step(by: bool, toks: &[&str]) -> Result<StepMsg, String> {
    let expr = toks.join(" ");
    crate::app::repro::query::parse_check(&expr)?;
    Ok(StepMsg::Check { by, expr })
}

/// `at T input STEP` (v2): one step of the `--input` grammar, checked
/// against it (points against a frame large enough for every play
/// resolution; the run checks them again against its own frame).
fn input_step(toks: &[&str]) -> Result<StepMsg, String> {
    let step = toks.join(" ");
    if step.is_empty() || step.contains(';') {
        return Err("`at T input <step>`: one step, no `;`".into());
    }
    if matches!(toks.first(), Some(&("frame" | "wait"))) {
        return Err(format!(
            "input {step:?}: the `at` tick times a step; `frame` and `wait` are not steps here"
        ));
    }
    crate::world_view::input_script::parse_step(&step, (4096, 4096))?;
    Ok(StepMsg::Input(step))
}

/// `char item <code> <place> [<key> <value>]...` (§2 item table).
fn item(args: &[&str]) -> Result<Item, String> {
    let [code, rest @ ..] = args else {
        return Err("`char item <code> <place>`".into());
    };
    if !(3..=4).contains(&code.len())
        || !code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err(format!("item code {code:?}: 3-4 of [a-z0-9]"));
    }
    let cell = |t: &str| ranged::<u8>(t, 0, 15, "grid cell");
    let (place, mut rest) = match rest {
        ["inv", x, y, r @ ..] => (Place::Inv(cell(x)?, cell(y)?), r),
        ["stash", x, y, r @ ..] => (Place::Stash(cell(x)?, cell(y)?), r),
        ["cube", x, y, r @ ..] => (Place::Cube(cell(x)?, cell(y)?), r),
        ["belt", s, r @ ..] => (Place::Belt(ranged(s, 0, 15, "belt slot")?), r),
        ["body", b, r @ ..] => (Place::Body(ranged(b, 1, 12, "body location")?), r),
        ["socket", n, r @ ..] => (Place::Socket(ranged(n, 0, 5, "socket")?), r),
        _ => {
            return Err(
                "item place: inv|stash|cube <x> <y>, belt <slot>, body <loc> or socket <n>".into(),
            )
        }
    };
    let mut it = Item::plain(code, place);
    fn once<T>(slot: &mut Option<T>, v: T, what: &str) -> Result<(), String> {
        if slot.is_some() {
            return Err(format!("item `{what}` given twice"));
        }
        *slot = Some(v);
        Ok(())
    }
    while let [key, value, r @ ..] = rest {
        let row = || ranged::<u16>(value, 0, 0xFFFF, key);
        match *key {
            "quality" => {
                let q = Quality::ALL
                    .into_iter()
                    .find(|q| q.name() == *value)
                    .ok_or_else(|| format!("unknown quality {value:?}"))?;
                once(&mut it.quality, q, "quality")?;
            }
            "ilvl" => once(&mut it.ilvl, ranged(value, 1, 99, "ilvl")?, "ilvl")?,
            "prefix" => it.prefixes.push(row()?),
            "suffix" => it.suffixes.push(row()?),
            "unique" => once(&mut it.unique, row()?, "unique")?,
            "set" => once(&mut it.set, row()?, "set")?,
            "runeword" => once(&mut it.runeword, row()?, "runeword")?,
            "sockets" => once(&mut it.sockets, ranged(value, 1, 6, "sockets")?, "sockets")?,
            k => return Err(format!("unknown item field {k:?}")),
        }
        rest = r;
    }
    if !rest.is_empty() {
        return Err(format!("item field {:?} without a value", rest[0]));
    }
    let q = it.quality;
    if it.prefixes.len() > 3 || it.suffixes.len() > 3 {
        return Err("at most 3 prefixes and 3 suffixes".into());
    }
    if !(it.prefixes.is_empty() && it.suffixes.is_empty())
        && !matches!(q, Some(Quality::Magic | Quality::Rare | Quality::Crafted))
    {
        return Err("prefix / suffix need quality magic, rare or crafted".into());
    }
    if it.unique.is_some() != (q == Some(Quality::Unique)) {
        return Err("`unique <id>` and quality unique go together".into());
    }
    if it.set.is_some() != (q == Some(Quality::Set)) {
        return Err("`set <id>` and quality set go together".into());
    }
    if it.runeword.is_some() && it.sockets.is_none() {
        return Err("a runeword needs `sockets`".into());
    }
    Ok(it)
}

/// `spawn <class> <x> <y> <kind> [free] [umod <id>...]` (§3.1).
fn spawn(toks: &[&str]) -> Result<StepMsg, String> {
    let [class, x, y, kind, rest @ ..] = toks else {
        return Err("`spawn <class> <x> <y> <kind> [free] [umod <id>...]`".into());
    };
    let arg = |t: &str| -> Result<Arg, String> {
        if t.starts_with('@') {
            Ok(Arg::Ref(parse_ref(t)?))
        } else {
            Ok(Arg::Num(num(t)?))
        }
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
            .map(|t| ranged::<u8>(t, 1, 255, "umod"))
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
    Ok(StepMsg::Spawn(Spawn {
        class: num(class)?,
        x: arg(x)?,
        y: arg(y)?,
        kind,
        free,
        umods,
    }))
}

/// `hex <byte>...`: 1 to [`MAX_HEX`] bytes, two hex digits each.
fn hex(toks: &[&str]) -> Result<StepMsg, String> {
    let b = toks
        .iter()
        .map(|t| {
            if t.len() == 2 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
                u8::from_str_radix(t, 16).map_err(|e| e.to_string())
            } else {
                Err(format!("hex byte {t:?}: two hex digits"))
            }
        })
        .collect::<Result<Vec<u8>, _>>()?;
    if b.is_empty() || b.len() > MAX_HEX {
        return Err(format!("hex message of {} bytes: 1..={MAX_HEX}", b.len()));
    }
    Ok(StepMsg::Hex(b))
}

/// One message outside a script (the tools' `send` steps,
/// `tools/scenario-diff.md` §2): `hex <byte>...` (a raw message, as an
/// `at … hex` step) or `<Name> <field>=<value>...` (a typed message, as
/// an `at … msg` step; §3 rules 1–3). The same strict errors as a step.
pub fn parse_message(toks: &[&str]) -> Result<StepMsg, String> {
    match toks.split_first() {
        Some((&"hex", bytes)) => hex(bytes),
        Some(_) => typed(toks),
        None => Err("a message: `hex <byte>...` or `<Name> <field>=<value>...`".into()),
    }
}

/// The canonical text of a message step's body (§2 rule 6): `hex …` or
/// `<Name> <field>=<value>...` in layout order; `None` for a spawn or
/// poke step.
pub fn message_text(msg: &StepMsg) -> Option<String> {
    match msg {
        StepMsg::Hex(b) => Some(
            std::iter::once("hex".to_owned())
                .chain(b.iter().map(|x| format!("{x:02x}")))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        StepMsg::Typed { id, fields } => {
            let mut t = message(*id).name.to_owned();
            for (n, a) in fields {
                match a {
                    Arg::Num(v) => t.push_str(&format!(" {n}={v}")),
                    Arg::Ref(r) => t.push_str(&format!(" {n}={r}")),
                }
            }
            Some(t)
        }
        StepMsg::Checkpoint
        | StepMsg::ConditionalKey { .. }
        | StepMsg::Spawn(_)
        | StepMsg::Poke(_)
        | StepMsg::Dev(_)
        | StepMsg::Input(_)
        | StepMsg::Check { .. } => None,
    }
}

/// `msg <Name> <field>=<value>...` (§3 rules 1–2).
fn typed(toks: &[&str]) -> Result<StepMsg, String> {
    let Some((&name, assigns)) = toks.split_first() else {
        return Err("`msg` needs a message name".into());
    };
    let m = find_message(name).ok_or_else(|| format!("unknown message {name:?}"))?;
    typed_layout(m)?;
    let mut given: Vec<(&str, Arg)> = Vec::new();
    for a in assigns {
        let (n, v) = a
            .split_once('=')
            .ok_or_else(|| format!("{a:?}: <field>=<value>"))?;
        let f = m
            .layout
            .iter()
            .find(|f| f.name == n)
            .ok_or_else(|| format!("{name} has no field {n:?}"))?;
        if given.iter().any(|(g, _)| *g == n) {
            return Err(format!("field {n} given twice"));
        }
        let v = if v.starts_with('@') {
            Arg::Ref(parse_ref(v)?)
        } else {
            let x = num(v)?;
            if x > field_max(f.ty) {
                return Err(format!("{n}={x} does not fit the field"));
            }
            Arg::Num(x)
        };
        given.push((n, v));
    }
    let mut fields = Vec::with_capacity(m.layout.len());
    for f in m.layout {
        let Some(i) = given.iter().position(|(g, _)| *g == f.name) else {
            return Err(format!("{name} needs field {}", f.name));
        };
        fields.push((f.name.to_owned(), given.swap_remove(i).1));
    }
    Ok(StepMsg::Typed { id: m.id, fields })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct W;
    impl World for W {
        fn player(&self) -> Option<(u32, i32, i32)> {
            Some((7, 100, 200))
        }
        fn units(&self) -> Vec<UnitRef> {
            let u = |ty, class, guid, pos, waypoint| UnitRef {
                ty,
                class,
                guid,
                pos,
                waypoint,
            };
            vec![
                u(0, 1, 7, Some((100, 200)), false),
                u(1, 148, 30, Some((100, 210)), false),
                u(1, 148, 12, Some((100, 250)), false),
                u(2, 0, 5, Some((90, 200)), true),
                u(2, 9, 4, Some((110, 200)), false),
            ]
        }
    }

    const BASE: &str = "scenario 1\nname t\ngame 1.14d\nseed 1\ninit 2\ndifficulty normal\nexpansion yes\nend 10\nchar class 1\nchar area 0 1\n";

    fn steps(extra: &str) -> Result<Scenario, ScriptError> {
        Scenario::parse(&format!("{BASE}{extra}"))
    }

    fn bytes(step: &str) -> Vec<u8> {
        let s = steps(&format!("{step}\n")).unwrap();
        encode(&s.steps[0].msg, &W).unwrap()
    }

    // Covers: specs/tools/scenario.md §3 r2
    #[test]
    fn typed_messages_encode_from_the_layout() {
        assert_eq!(bytes("at 0 msg Walk x=10 y=20"), [1, 10, 0, 20, 0]);
        assert_eq!(bytes("at 0 msg Walk y=20 x=10"), [1, 10, 0, 20, 0]);
        assert_eq!(
            bytes("at 0 msg SelectSkill skill=36 left=0 item=0xFFFFFFFF"),
            [0x3C, 36, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(
            bytes("at 0 msg SelectSkill skill=36 left=1 item=0"),
            [0x3C, 36, 0, 0, 0x80, 0, 0, 0, 0]
        );
        assert_eq!(bytes("at 0 hex 01 02"), [1, 2]);
    }

    // Covers: specs/tools/scenario.md §3 r3, §3 r4, §3 r5
    #[test]
    fn references_resolve_in_guid_order() {
        assert_eq!(bytes("at 0 msg Walk x=@x+5 y=@y-3"), [1, 105, 0, 197, 0]);
        assert_eq!(
            bytes("at 0 msg InteractWithEntity type=1 id=@1:148"),
            [0x13, 1, 0, 0, 0, 12, 0, 0, 0]
        );
        assert_eq!(
            bytes("at 0 msg InteractWithEntity type=1 id=@1:148#1"),
            [0x13, 1, 0, 0, 0, 30, 0, 0, 0]
        );
        assert_eq!(
            bytes("at 0 msg InteractWithEntity type=2 id=@wp"),
            [0x13, 2, 0, 0, 0, 5, 0, 0, 0]
        );
        assert_eq!(
            bytes("at 0 msg InteractWithEntity type=0 id=@player"),
            [0x13, 0, 0, 0, 0, 7, 0, 0, 0]
        );
        let s = steps("at 0 msg InteractWithEntity type=1 id=@1:148#2\n").unwrap();
        assert_eq!(
            encode(&s.steps[0].msg, &W),
            Err(Unresolved {
                reference: "@1:148#2".into(),
                why: "@1:148#2: no such unit".into()
            })
        );
        let s = steps("at 0 msg Walk x=@x-101 y=0\n").unwrap();
        let e = encode(&s.steps[0].msg, &W).unwrap_err();
        assert_eq!(e.reference, "@x-101");
        assert!(e.why.contains("does not fit"), "{e:?}");
    }

    // Covers: specs/tools/scenario.md §3 r1, §3 r2, §3 r3
    /// A message outside a script (`send` steps): the step's own parser
    /// and encoder, and its canonical text.
    #[test]
    fn single_messages_parse_as_steps() {
        let one = |t: &str| {
            let toks: Vec<&str> = t.split_whitespace().collect();
            parse_message(&toks)
        };
        let m = one("Walk y=20 x=10").unwrap();
        assert_eq!(encode(&m, &W).unwrap(), [1, 10, 0, 20, 0]);
        assert_eq!(message_text(&m).unwrap(), "Walk x=10 y=20");
        let m = one("SelectSkill skill=36 left=0 item=0xFFFFFFFF").unwrap();
        assert_eq!(
            encode(&m, &W).unwrap(),
            [0x3C, 36, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        let m = one("hex 13 01 00 00 00 0c 00 00 00").unwrap();
        assert_eq!(encode(&m, &W).unwrap(), [0x13, 1, 0, 0, 0, 12, 0, 0, 0]);
        assert_eq!(message_text(&m).unwrap(), "hex 13 01 00 00 00 0c 00 00 00");
        let m = one("BuyItem npc=@1:148 item=@4#1 transaction=0 client_price=0").unwrap();
        assert_eq!(
            message_text(&m).unwrap(),
            "BuyItem npc=@1:148 item=@4#1 transaction=0 client_price=0"
        );
        for bad in [
            "",
            "hex",
            "hex 1",
            "Walk x=1",
            "Nope a=1",
            "Walk x=70000 y=0",
            "Chat",
        ] {
            assert!(one(bad).is_err(), "{bad:?}");
        }
    }

    // Covers: specs/tools/scenario.md §2 r2, §2 r3, §2 r5, §3 r1
    #[test]
    fn strict_errors_name_the_line() {
        let cases: &[(&str, usize, &str)] = &[
            ("bogus 1\n", 11, "unknown keyword"),
            ("seed 2\n", 11, "given twice"),
            ("char class 2\n", 11, "given twice"),
            ("at 5 hex 01\nat 4 hex 01\n", 12, "after a step at tick 5"),
            ("at 11 hex 01\n", 11, "after end"),
            ("at 1 msg Walk x=1\n", 11, "needs field y"),
            ("at 1 msg Walk x=1 y=70000\n", 11, "does not fit"),
            ("at 1 msg Walk x=1 y=2 z=3\n", 11, "no field"),
            ("at 1 msg Nope\n", 11, "unknown message"),
            ("at 1 hex 1\n", 11, "two hex digits"),
            ("at 1 hex 01\nseed 3\n", 12, "after the first"),
            ("record c2s\n", 11, "always recorded"),
            ("record s2c s2c\n", 11, "twice"),
            ("char item abcde inv 0 0\n", 11, "item code"),
            ("char level 100\n", 11, "level 100"),
            ("at 1 msg Walk x=@z y=1\n", 11, "bad number"),
            ("at 1 msg Walk x=@6 y=1\n", 11, "unit type"),
            (
                "at 1 msg Walk x=1 y=2 # trailing comment\nat 1 msg Walk x=1 y=2#3\n",
                12,
                "bad number",
            ),
            ("at 1 msg Walk x=@x+0 y=1\n", 11, "zero offset"),
        ];
        for (extra, line, needle) in cases {
            let e = steps(extra).unwrap_err();
            assert_eq!(e.line, *line, "{extra:?}: {e}");
            assert!(e.message.contains(needle), "{extra:?}: {e}");
        }
        assert_eq!(
            Scenario::parse("scenario 3\n").unwrap_err().message,
            "unknown scenario format version 3"
        );
        let e = Scenario::parse("scenario 1\nname t\n").unwrap_err();
        assert_eq!((e.line, e.message.as_str()), (0, "missing `game` line"));
        let e = steps("snapshot every 5\n").unwrap_err();
        assert!(e.message.contains("need `units` or `stats`"), "{e}");
        let e = steps("char save x.d2s\n").unwrap_err();
        assert!(e.message.contains("character name"), "{e}");
        let s = steps("char save Scn_Sor\n").unwrap();
        assert_eq!(s.save.as_deref(), Some("Scn_Sor"));
        assert!(s.character.is_some());
        let e = Scenario::parse("scenario 1\nname t\ngame 1.14d\nseed 0x80000000\n").unwrap_err();
        assert!(e.message.contains("seed"), "{e}");
    }

    // Covers: specs/tools/scenario.md §3.1 r1, §3.1 r2, §3.1 r3
    #[test]
    fn spawn_steps_and_rich_items() {
        let s = steps(concat!(
            "char quest 3 0x1001\n",
            "char item rin1 inv 0 0 quality rare ilvl 85 prefix 12 suffix 400 suffix 401\n",
            "char item ber socket 0\n",
            "char item uap body 1 quality unique unique 230 sockets 1\n",
            "char item 7cr body 4 runeword 42 sockets 2 quality normal\n",
            "at 1 spawn 19 @x+10 @y champion umod 16\n",
            "at 1 spawn 5 100 200 unique umod 3 7 18\n",
            "at 2 spawn 63 @x @y-5 random-boss\n",
            "at 3 spawn 5 100 200 normal free\n",
        ))
        .unwrap();
        let text = s.to_text();
        assert_eq!(Scenario::parse(&text).unwrap(), s, "{text}");
        assert!(
            text.contains("char item 7cr body 4 quality normal runeword 42 sockets 2\n"),
            "{text}"
        );
        assert!(
            text.contains("at 1 spawn 19 @x+10 @y champion umod 16\n"),
            "{text}"
        );
        assert!(
            text.contains("at 3 spawn 5 100 200 normal free\n"),
            "{text}"
        );
        let StepMsg::Spawn(sp) = &s.steps[0].msg else {
            panic!()
        };
        assert_eq!(spawn_position(sp, &W), Ok((110, 200)));
        assert!(encode(&s.steps[0].msg, &W).is_err());
        let c = s.character.as_ref().unwrap();
        assert_eq!(c.quests, [(3, 0x1001)]);
        assert_eq!(c.items[1].place, Place::Socket(0));
        for (bad, needle) in [
            ("at 1 spawn 19 1 2 champion\n", "one for champion"),
            ("at 1 spawn 19 1 2 normal umod 3\n", "none for normal"),
            ("at 1 spawn 19 1 2 elite\n", "unknown spawn kind"),
            (
                "char item rin1 inv 0 0 prefix 3\n",
                "magic, rare or crafted",
            ),
            ("char item rin1 inv 0 0 quality unique\n", "go together"),
            ("char item 7cr inv 0 0 runeword 3\n", "needs `sockets`"),
            ("char item ber socket 0\n", "earlier item"),
            ("char item rin1 inv 0 0 ilvl\n", "without a value"),
            ("char quest 41 1\n", "quest index"),
        ] {
            let e = steps(bad).unwrap_err();
            assert!(e.message.contains(needle), "{bad:?}: {e}");
        }
    }

    // Covers: specs/tools/poke.md §3 r1, §3 r2
    #[test]
    fn poke_steps_and_the_variant_line_round_trip() {
        let s = steps(concat!(
            "variant no-ambient\n",
            "at 1 spawn 19 @x+3 @y normal\n",
            "at 1 poke seed-unit @1:19 0x10 2\n",
            "at 2 msg Walk x=@x y=@y\n",
            "at 2 poke missile 7 @x @y @x+10 @y owner @player skill 36 1\n",
            "at 3 poke item hp1 @x @y ilvl 5 quality magic\n",
        ))
        .unwrap();
        assert_eq!(s.variant.as_deref(), Some("no-ambient"));
        let text = s.to_text();
        assert_eq!(Scenario::parse(&text).unwrap(), s, "{text}");
        assert!(
            text.contains("end 10\nvariant no-ambient\nchar class"),
            "{text}"
        );
        assert!(text.contains("at 1 poke seed-unit @1:19 16 2\n"), "{text}");
        assert!(
            text.contains("at 2 poke missile 7 @x @y @x+10 @y skill 36 1 owner @player\n"),
            "{text}"
        );
        assert!(
            text.contains("at 3 poke item hp1 @x @y quality magic ilvl 5\n"),
            "{text}"
        );
        assert!(encode(&s.steps[1].msg, &W).is_err());
        // The variant is part of the canonical text, so of the digest.
        let base = steps("").unwrap();
        assert_ne!(
            base.sha256(),
            steps("variant no-ambient\n").unwrap().sha256()
        );
        assert_eq!(base.variant, None);
        for (bad, needle) in [
            ("at 1 poke\n", "empty directive"),
            ("at 1 poke teleport 1 2\n", "unknown directive"),
            ("at 1 poke time 7 0\n", "period 7"),
            ("at 1 poke time 1\n", "takes 2"),
            ("at 1 poke object 1 2 3 4\n", "unexpected"),
            ("variant A\n", "variant"),
            ("variant a\nvariant b\n", "given twice"),
        ] {
            let e = steps(bad).unwrap_err();
            assert_eq!(e.line, 11 + bad.matches('\n').count() - 1, "{bad:?}: {e}");
            assert!(e.message.contains(needle), "{bad:?}: {e}");
        }
    }

    #[test]
    fn conditional_player_keys_validate_and_round_trip() {
        let text = "scenario 2\ntick_rate 60\nname key\nend 4\nchar class 0\nat 2 key-if 32 client.dialog_lines != null\n";
        let scenario = Scenario::parse(text).unwrap();
        assert_eq!(Scenario::parse(&scenario.to_text()).unwrap(), scenario);
        assert!(Scenario::parse(&text.replace("key-if 32", "key-if 0")).is_err());
        assert!(Scenario::parse(&text.replace("client.dialog_lines", "no_such_name")).is_err());
    }

    #[test]
    fn checkpoints_are_v2_steps_with_canonical_text() {
        let text =
            "scenario 2\ntick_rate 60\nname checkpoint\nend 12\nchar class 0\nat 6 checkpoint\n";
        let parsed = Scenario::parse(text).unwrap();
        assert_eq!(parsed.steps[0].msg, StepMsg::Checkpoint);
        assert_eq!(Scenario::parse(&parsed.to_text()).unwrap(), parsed);
        for bad in [
            text.replace("at 6 checkpoint", "by 6 checkpoint"),
            text.replace("at 6 checkpoint", "at 6 checkpoint extra"),
        ] {
            assert!(Scenario::parse(&bad).is_err());
        }
    }

    // Covers legacy snapshot interval expansion and canonical seed formatting.
    #[test]
    fn canonical_text_round_trips() {
        let s = steps(
            "record units s2c\nsnapshot every 25\nsnapshot at 3 1\nchar stat 0 -5\nchar item hp1 belt 2\nat 1 msg Walk x=@x+1 y=@y\nat 1 hex 6d 00\n",
        )
        .unwrap();
        let text = s.to_text();
        let again = Scenario::parse(&text).unwrap();
        assert_eq!(again, s);
        assert_eq!(again.to_text(), text);
        assert_eq!(
            s.snapshot_ticks().into_iter().collect::<Vec<_>>(),
            [0, 1, 3, 10]
        );
        assert!(text.contains("seed 0x00000001\n"), "{text}");
        assert!(text.contains("at 1 msg Walk x=@x+1 y=@y\n"), "{text}");
    }
    // Covers: specs/tools/scenario.md §3 row1, §3 row2, §3 row3, §3 row4, §3 row5
    #[test]
    fn every_reference_form_resolves() {
        let one = |r: &str| resolve(&parse_ref(r).unwrap(), &W);
        assert_eq!(one("@player"), Ok(7));
        assert_eq!(one("@x"), Ok(100));
        assert_eq!(one("@y"), Ok(200));
        assert_eq!(one("@x+7"), Ok(107));
        assert_eq!(one("@x-7"), Ok(93));
        assert_eq!(one("@y+7"), Ok(207));
        assert_eq!(one("@y-7"), Ok(193));
        // @<type>, @<type>#n: ascending GUID order over all units of the type.
        assert_eq!(one("@1"), Ok(12));
        assert_eq!(one("@1#1"), Ok(30));
        assert!(one("@1#2").is_err());
        assert_eq!(one("@0"), Ok(7));
        // @<type>:<class>, with #n.
        assert_eq!(one("@2:9"), Ok(4));
        assert_eq!(one("@1:148#1"), Ok(30));
        assert!(one("@2:9#1").is_err());
        // @near orders by distance to the player, then GUID.
        assert_eq!(one("@1:148@near"), Ok(30));
        assert_eq!(one("@1:148#1@near"), Ok(12));
        assert!(one("@1:148#2@near").is_err());
        assert_eq!(one("@2:9@near"), Ok(4));
        // The reference round-trips through the scenario text.
        let text = "at 1 msg InteractWithEntity type=1 id=@1:148#1@near\n";
        let s = steps(text).unwrap();
        assert!(s.to_text().contains(text), "{}", s.to_text());
        // @wp picks only waypoint objects.
        assert_eq!(one("@wp"), Ok(5));
        assert!(one("@wp#1").is_err());
    }

    // Covers: specs/tools/scenario.md §3.1 row1, §3.1 row2, §3.1 row3, §3.1 row4, §3.1 r4
    #[test]
    fn spawn_kinds_take_their_umod_counts_and_unresolved_refs_spawn_nothing() {
        let ok = |kind: &str, umods: &str| steps(&format!("at 1 spawn 19 1 2 {kind}{umods}\n"));
        assert!(ok("normal", "").is_ok());
        assert!(ok("random-boss", "").is_ok());
        assert!(ok("champion", " umod 16").is_ok());
        assert!(ok("champion", " umod 16 17").is_err());
        assert!(ok("champion", "").is_err());
        assert!(ok("random-boss", " umod 1").is_err());
        assert!(ok("unique", " umod 1").is_ok());
        assert!(ok("unique", " umod 1 2 3 4 5 6 7 8 9").is_ok());
        assert!(ok("unique", " umod 1 2 3 4 5 6 7 8 9 10").is_err());
        assert!(ok("unique", "").is_err());
        // Unresolved reference: no position, nothing is spawned (§3 rule 5).
        let s = steps("at 1 spawn 19 @1:999 @y normal\n").unwrap();
        let StepMsg::Spawn(sp) = &s.steps[0].msg else {
            panic!()
        };
        let e = spawn_position(sp, &W).unwrap_err();
        assert_eq!(e.reference, "@1:999");
    }

    const V2: &str = "scenario 2\nname t\nend 50\nchar class 1\n";

    fn v2(extra: &str) -> Result<Scenario, ScriptError> {
        Scenario::parse(&format!("{V2}{extra}"))
    }

    #[test]
    fn v2_header_defaults_and_inline_character() {
        let s = v2("").unwrap();
        assert_eq!(s.version, SCENARIO_FORMAT_V2);
        assert_eq!(
            (s.seed, s.init, s.difficulty, s.expansion, s.end),
            (V2_DEFAULT_SEED, 0, Difficulty::Normal, true, 50)
        );
        let c = s.character.unwrap();
        assert_eq!((c.class, c.level, c.act, c.area), (1, 1, 0, 1));
        assert_eq!(c.hands, [None, None]);
        let s = v2(
            "seed 7\ndifficulty hell\nexpansion no\nchar level 30\nchar hand right 47\nchar skill 47 20\n",
        )
        .unwrap();
        assert_eq!(
            (s.seed, s.difficulty, s.expansion),
            (7, Difficulty::Hell, false)
        );
        let c = s.character.unwrap();
        assert_eq!(
            (c.level, c.hands, c.skills.as_slice()),
            (30, [None, Some(47)], &[(47, 20)][..])
        );
    }

    #[test]
    fn v2_save_file_dev_and_input_steps() {
        let s = Scenario::parse(
            "scenario 2\nname t\nend 40\nsave chars/Sor.d2s\nat 2 dev god on\nat 3 poke warp 2\nat 5 input rclickunit 1 *\nat 5 input click 400 300\n",
        )
        .unwrap();
        assert_eq!(s.save_file.as_deref(), Some("chars/Sor.d2s"));
        assert!(s.character.is_none());
        let kinds: Vec<(u32, String)> = s
            .steps
            .iter()
            .map(|st| {
                let k = match &st.msg {
                    StepMsg::Dev(c) => format!("dev {c}"),
                    StepMsg::Input(i) => format!("input {i}"),
                    StepMsg::Poke(d) => format!("poke {d}"),
                    other => format!("{other:?}"),
                };
                (st.tick, k)
            })
            .collect();
        assert_eq!(
            kinds,
            [
                (2, "dev god on".to_owned()),
                (3, "poke warp 2".to_owned()),
                (5, "input rclickunit 1 *".to_owned()),
                (5, "input click 400 300".to_owned()),
            ]
        );
        assert!(encode(&s.steps[0].msg, &W).is_err(), "not a message");
        assert_eq!(message_text(&s.steps[2].msg), None);
    }

    #[test]
    fn v2_errors_name_the_line() {
        let cases: &[(&str, usize, &str)] = &[
            ("game 1.14d\n", 5, "scenario 1"),
            ("char save Sor\n", 5, "save <file>"),
            ("char hand middle 47\n", 5, "left or right"),
            ("char hand left 47\nchar hand left 48\n", 6, "given twice"),
            ("at 1 dev nosuchcommand\n", 5, "nosuchcommand"),
            ("at 1 dev clear\n", 5, "console-only"),
            ("at 1 input fly 1 2\n", 5, "unknown step"),
            ("at 1 input frame 3\n", 5, "not steps here"),
            ("at 1 input wait 3\n", 5, "not steps here"),
            ("at 1 input click 1\n", 5, "needs X Y"),
            ("at 1 bogus 1\n", 5, "check or checkpoint"),
            ("at 1 check player.speed == 1\n", 5, "unknown unit field"),
            ("at 1 check\n", 5, "an empty name"),
            ("by 1 dev god on\n", 5, "by <tick> check"),
        ];
        for (extra, line, needle) in cases {
            let e = v2(extra).unwrap_err();
            assert_eq!(e.line, *line, "{extra:?}: {e}");
            assert!(e.message.contains(needle), "{extra:?}: {e}");
        }
        let e = Scenario::parse("scenario 2\nname t\nend 5\n").unwrap_err();
        assert!(e.message.contains("save <file>"), "{e}");
        let e = v2("save a.d2s\n").unwrap_err();
        assert!(e.message.contains("not both"), "{e}");
        let e = Scenario::parse("scenario 2\nname t\nchar class 1\n").unwrap_err();
        assert_eq!(e.message, "missing `end` line");
        // v1 keeps its own grammar: no dev / input steps, no `save` line.
        let e = steps("at 1 dev god on\n").unwrap_err();
        assert!(e.message.contains("spawn or poke"), "{e}");
        let e = steps("save a.d2s\n").unwrap_err();
        assert!(e.message.contains("unknown keyword"), "{e}");
        let e = steps("at 1 check hud.life == 1\n").unwrap_err();
        assert!(e.message.contains("spawn or poke"), "{e}");
        let e = steps("by 1 check hud.life == 1\n").unwrap_err();
        assert!(e.message.contains("scenario 2"), "{e}");
    }

    #[test]
    fn v2_check_lines_parse_and_round_trip() {
        let s = v2(
            "at 0 check ui.open(inventory)\nat 3 check player.pos ~ (5120, 3400) 2\nby 9 check ground_item(gold).label == \"1234 Gold\"\n",
        )
        .unwrap();
        let checks: Vec<(u32, bool, &str)> = s
            .steps
            .iter()
            .filter_map(|st| match &st.msg {
                StepMsg::Check { by, expr } => Some((st.tick, *by, expr.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            checks,
            [
                (0, false, "ui.open(inventory)"),
                (3, false, "player.pos ~ (5120, 3400) 2"),
                (9, true, "ground_item(gold).label == \"1234 Gold\""),
            ]
        );
        let text = s.to_text();
        assert!(
            text.contains("by 9 check ground_item(gold).label"),
            "{text}"
        );
        assert_eq!(Scenario::parse(&text).unwrap(), s);
        assert!(encode(&s.steps[0].msg, &W).is_err(), "not a message");
    }

    #[test]
    fn v2_canonical_text_round_trips_and_v1_stays_v1() {
        let s = v2(
            "char hand left 0\nchar hand right 47\nat 1 dev level 30\nat 2 input hold 400 300 5\n",
        )
        .unwrap();
        let text = s.to_text();
        assert!(text.starts_with("scenario 2\n"), "{text}");
        assert!(!text.contains("game "), "{text}");
        assert!(text.contains("char hand right 47\n"), "{text}");
        assert!(text.contains("at 1 dev level 30\n"), "{text}");
        assert!(text.contains("at 2 input hold 400 300 5\n"), "{text}");
        let again = Scenario::parse(&text).unwrap();
        assert_eq!(again, s);
        assert_eq!(again.to_text(), text);
        let one = steps("").unwrap();
        assert_eq!(one.version, SCENARIO_FORMAT_VERSION);
        assert!(one.to_text().starts_with("scenario 1\nname t\ngame "));
        let f = Scenario::parse("scenario 2\nname t\nend 5\nsave x.d2s\n").unwrap();
        assert_eq!(Scenario::parse(&f.to_text()).unwrap(), f);
    }
}

#[cfg(test)]
mod allow_tests {
    use super::*;
    #[test]
    fn v2_allow_roundtrips_and_changes_the_scenario_hash() {
        let text = "scenario 2\nname allows\nend 1\nchar class 0\n";
        let plain = Scenario::parse(text).unwrap();
        let allowed =
            Scenario::parse(&format!("{text}allow hud\nallow animation\nallow hud\n")).unwrap();
        assert_eq!(
            allowed.allow.iter().map(String::as_str).collect::<Vec<_>>(),
            ["animation", "hud"]
        );
        assert_eq!(Scenario::parse(&allowed.to_text()).unwrap(), allowed);
        assert_ne!(plain.sha256(), allowed.sha256());
        assert!(Scenario::parse(&format!("{text}allow typo\n"))
            .unwrap_err()
            .message
            .contains("unknown check rule"));
        assert!(Scenario::parse(&format!("{text}allow hud extra\n")).is_err());
    }
    #[test]
    fn v1_rejects_allow() {
        let error = Scenario::parse("scenario 1\nallow hud\n").unwrap_err();
        assert_eq!(error.line, 2);
    }
}
