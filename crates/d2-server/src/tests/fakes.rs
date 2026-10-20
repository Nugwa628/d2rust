//! Test fakes for the `d2-server` seams, shared by every file in this test suite.
//! `TsvSizes` evaluates the size-rule grammar (`u8@off`, `u16@off`, `*mul`, `+add`,
//! `cap=`, `min=`, plus the `chat`, `chat26` and `af` special rules) over the rows
//! of `client-messages.tsv` / `server-messages.tsv` (stand-in for `d2-proto`).
//! `FakeGame` stands in for `d2-sim`, and `FakeSession` and `ManualClock` stand in
//! for the session handler and the clock. `adapters.rs` checks the fakes against
//! the real adapters.

use std::collections::BTreeMap;

use crate::seams::*;

/// Client-to-server message table (ids, transport sizes) from `tables/sim`.
pub const CLIENT_TSV: &str = include_str!("../../../../tables/sim/client-messages.tsv");
/// Server-to-client message table (ids, sizes) from `tables/sim`.
pub const SERVER_TSV: &str = include_str!("../../../../tables/sim/server-messages.tsv");

/// Rows of a TSV as (header → cell) maps, in file order.
pub fn rows(tsv: &str) -> Vec<BTreeMap<String, String>> {
    let mut lines = tsv.lines();
    let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
    lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            header
                .iter()
                .zip(l.split('\t'))
                .map(|(h, c)| (h.to_string(), c.to_string()))
                .collect()
        })
        .collect()
}

/// Parses a hex cell such as `0x15` (the prefix is optional).
pub fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

/// A parsed size rule from the `size` / `transport_size` columns.
#[derive(Clone, Debug)]
pub enum Rule {
    /// A constant size; 0 means the id is not a valid message.
    Fixed(usize),
    /// Size read from the message: `value * mul + add`, where the value is the
    /// byte (or little-endian u16 if `wide`) at `off`.
    Field {
        /// The length field is a u16 instead of a u8.
        wide: bool,
        /// Byte offset of the length field.
        off: usize,
        /// Factor applied to the field value.
        mul: usize,
        /// Constant added after the multiplication.
        add: usize,
        /// Field values above this give size 0 (invalid).
        cap: Option<usize>,
        /// Bytes that must be present before the field is read.
        min: usize,
    },
    /// Chat message: two NUL-terminated strings plus a signed size adjustment byte.
    Chat,
    /// Chat message with a fixed 10-byte head followed by two NUL-terminated strings.
    Chat26,
    /// Size is byte 1 plus one, or 2 when byte 1 is 0.
    Af,
}

/// Parses a rule cell such as `u16@1*2+3;cap=40;min=4`, `chat` or a plain number.
pub fn parse_rule(s: &str) -> Rule {
    match s {
        "chat" => return Rule::Chat,
        "chat26" => return Rule::Chat26,
        "af" => return Rule::Af,
        _ => {}
    }
    if let Ok(n) = s.parse() {
        return Rule::Fixed(n);
    }
    let num = |v: &str| -> usize {
        if let Some(h) = v.strip_prefix("0x") {
            usize::from_str_radix(h, 16).unwrap()
        } else {
            v.parse().unwrap()
        }
    };
    let mut parts = s.split(';');
    let expr = parts.next().unwrap();
    let (mut cap, mut min) = (None, 0);
    for p in parts {
        let (k, v) = p.split_once('=').unwrap();
        match k {
            "cap" => cap = Some(num(v)),
            "min" => min = num(v),
            _ => panic!("rule option {k}"),
        }
    }
    let (ty, rest) = expr.split_once('@').unwrap();
    let wide = match ty {
        "u8" => false,
        "u16" => true,
        _ => panic!("rule type {ty}"),
    };
    let (mut off_s, mut mul, mut add) = (rest, 1, 0);
    if let Some((a, b)) = off_s.split_once('+') {
        off_s = a;
        add = num(b);
    }
    if let Some((a, b)) = off_s.split_once('*') {
        off_s = a;
        mul = num(b);
    }
    Rule::Field {
        wide,
        off: num(off_s),
        mul,
        add,
        cap,
        min,
    }
}

fn cstr(b: &[u8], at: usize) -> Result<usize, SizeError> {
    b.get(at..)
        .and_then(|s| s.iter().position(|&c| c == 0))
        .ok_or(SizeError::Incomplete)
}

/// Evaluates a rule on a message prefix: its size, or why none can be given.
pub fn eval(rule: &Rule, b: &[u8]) -> Result<usize, SizeError> {
    let need = |n: usize| {
        if b.len() >= n {
            Ok(())
        } else {
            Err(SizeError::Incomplete)
        }
    };
    match *rule {
        Rule::Fixed(0) => Err(SizeError::Invalid),
        Rule::Fixed(n) => Ok(n),
        Rule::Field {
            wide,
            off,
            mul,
            add,
            cap,
            min,
        } => {
            need(min.max(off + if wide { 2 } else { 1 }))?;
            let mut v = if wide {
                usize::from(u16::from_le_bytes([b[off], b[off + 1]]))
            } else {
                usize::from(b[off])
            };
            if cap.is_some_and(|c| v > c) {
                v = 0;
            }
            // A rule giving 0 is "not a valid message".
            match v * mul + add {
                0 => Err(SizeError::Invalid),
                n => Ok(n),
            }
        }
        Rule::Chat => {
            need(3)?;
            let l1 = cstr(b, 3)?;
            let l2 = cstr(b, l1 + 4)?;
            need(l1 + l2 + 6)?;
            let c = b[l1 + l2 + 5] as i8 as isize;
            let size = (l1 + l2 + 6) as isize + c;
            // "needs >= size" is left to the classifier's "> the given
            // size" test (same result 3), as in `check_packets.py`. A
            // negative size is its own result.
            match size {
                0 => Err(SizeError::Invalid),
                n if n < 0 => Err(SizeError::Negative(n as i32)),
                n => Ok(n as usize),
            }
        }
        Rule::Chat26 => {
            need(10)?;
            let a = cstr(b, 10)?;
            let c = cstr(b, 10 + a + 1)?;
            Ok(10 + a + 1 + c + 1)
        }
        Rule::Af => {
            need(2)?;
            Ok(if b[1] == 0 { 2 } else { usize::from(b[1]) + 1 })
        }
    }
}

/// Size rules read from both TSVs.
pub struct TsvSizes {
    /// Rule per client-to-server id, indexed by id.
    pub client: Vec<Rule>,
    /// Rule per server-to-client id, indexed by id.
    pub server: Vec<Rule>,
}

impl TsvSizes {
    /// Loads both tables and checks that their ids are in order.
    pub fn new() -> Self {
        let load = |tsv, col: &str| -> Vec<Rule> {
            rows(tsv)
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    assert_eq!(hex(&r["id"]) as usize, i, "ids in order");
                    parse_rule(&r[col])
                })
                .collect()
        };
        Self {
            client: load(CLIENT_TSV, "transport_size"),
            server: load(SERVER_TSV, "size"),
        }
    }
}

impl MessageSizes for TsvSizes {
    fn client_size(&self, msg: &[u8]) -> Result<usize, SizeError> {
        let rule = self
            .client
            .get(usize::from(msg[0]))
            .ok_or(SizeError::Invalid)?;
        eval(rule, msg)
    }
    fn server_size(&self, msg: &[u8]) -> Result<usize, SizeError> {
        let rule = self
            .server
            .get(usize::from(msg[0]))
            .ok_or(SizeError::Invalid)?;
        eval(rule, msg)
    }
}

/// A player in the fake game: its input gate and its point-message state.
#[derive(Clone, Debug)]
pub struct FakePlayer {
    /// Unit mode and state the dispatch gate looks at.
    pub gate: PlayerGate,
    /// Position and last-accept frame used by point-message parsing.
    pub point: Option<PointState>,
}

/// A game: players by client, units by id, a client list, and logs of
/// handler calls and ticks.
#[derive(Default)]
pub struct FakeGame {
    /// Current frame; each tick adds one.
    pub frame: i32,
    /// Players by client: `None` entry is a client without a player unit.
    pub players: BTreeMap<ClientId, Option<FakePlayer>>,
    /// Units a message can target, by (unit type, unit id).
    pub units: BTreeMap<(u32, u32), UnitTarget>,
    /// Clients in the order the game lists them.
    pub client_list: Vec<ClientId>,
    /// Handler calls seen: (client, message, size).
    pub handled: Vec<(ClientId, Vec<u8>, usize)>,
    /// Clients that were queued a resync.
    pub resyncs: Vec<ClientId>,
    /// Messages each tick queues: (client, bytes).
    pub tick_out: Vec<(ClientId, Vec<u8>)>,
    /// Messages each handler call queues.
    pub handler_out: Vec<(ClientId, Vec<u8>)>,
    /// System ids the game's session part takes (`Intents::session_message`),
    /// each answered with S→C 0x02; the rest go to the `SessionHandler`.
    pub session_ids: Vec<u8>,
    /// The system messages the game took.
    pub session_seen: Vec<(ClientId, Vec<u8>, usize)>,
    /// The game has mod rules: it takes mod messages
    /// (`Intents::mod_message`) and answers each with its payload
    /// reversed, in an S→C envelope.
    pub mod_rules: bool,
    /// The mod message payloads the game took.
    pub mod_seen: Vec<(ClientId, Vec<u8>)>,
    /// The game's tick rate (the default is the base rate).
    pub rate: d2_sim::tick::TickRate,
}

/// A living, interruptible player (mode 1).
pub const ALIVE: PlayerGate = PlayerGate {
    mode: 1,
    uninterruptable: false,
};

impl FakeGame {
    /// A game with one client who has a player at (100, 100).
    pub fn with_player(client: ClientId, gate: PlayerGate) -> Self {
        let mut g = Self::default();
        g.players.insert(
            client,
            Some(FakePlayer {
                gate,
                point: Some(PointState {
                    player: Pos { x: 100, y: 100 },
                    last_accept: 0,
                }),
            }),
        );
        g.client_list.push(client);
        g
    }
}

impl Intents for FakeGame {
    fn player(&self, client: ClientId) -> PlayerLookup {
        match self.players.get(&client) {
            None => PlayerLookup::NotInGame,
            Some(None) => PlayerLookup::NoPlayer,
            Some(Some(p)) => PlayerLookup::Player(p.gate),
        }
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn point_state(&self, client: ClientId) -> Option<PointState> {
        self.players.get(&client)?.as_ref()?.point
    }
    fn set_point_accept(&mut self, client: ClientId, frame: i32) {
        let p = self.players.get_mut(&client).unwrap().as_mut().unwrap();
        p.point.as_mut().unwrap().last_accept = frame;
    }
    fn queue_resync(&mut self, client: ClientId, out: &mut dyn MessageSink) {
        self.resyncs.push(client);
        out.queue(client, &[0x15; 11]).unwrap();
    }
    fn unit_target(&self, _client: ClientId, unit_type: u32, unit_id: u32) -> UnitTarget {
        self.units
            .get(&(unit_type, unit_id))
            .copied()
            .unwrap_or(UnitTarget::Missing)
    }
    fn handle(
        &mut self,
        client: ClientId,
        msg: &[u8],
        size: usize,
        out: &mut dyn MessageSink,
    ) -> ResultCode {
        self.handled.push((client, msg.to_vec(), size));
        for (c, m) in &self.handler_out {
            out.queue(*c, m).unwrap();
        }
        ResultCode::Done
    }
    fn clients(&self) -> Vec<ClientId> {
        self.client_list.clone()
    }
    fn session_message(
        &mut self,
        client: ClientId,
        msg: &[u8],
        size: usize,
        out: &mut dyn MessageSink,
    ) -> bool {
        if !self.session_ids.contains(&msg[0]) {
            return false;
        }
        self.session_seen.push((client, msg.to_vec(), size));
        out.queue(client, &[0x02]).unwrap();
        true
    }
    fn mod_message(&mut self, client: ClientId, payload: &[u8], out: &mut dyn MessageSink) -> bool {
        if !self.mod_rules {
            return false;
        }
        self.mod_seen.push((client, payload.to_vec()));
        let reply: Vec<u8> = payload.iter().rev().copied().collect();
        let msg = d2_proto::envelope::encode(d2_proto::envelope::Direction::ToClient, &reply);
        out.queue(client, &msg.unwrap()).unwrap();
        true
    }
}

impl Tick for FakeGame {
    fn tick(&mut self, out: &mut dyn MessageSink) {
        self.frame += 1;
        for (c, m) in &self.tick_out {
            out.queue(*c, m).unwrap();
        }
    }

    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.rate
    }
}

/// Records system messages.
#[derive(Default)]
pub struct FakeSession {
    /// System messages received: (client, message, size).
    pub seen: Vec<(ClientId, Vec<u8>, usize)>,
}

impl SessionHandler for FakeSession {
    fn system_message(
        &mut self,
        client: ClientId,
        msg: &[u8],
        size: usize,
        _out: &mut dyn MessageSink,
    ) {
        self.seen.push((client, msg.to_vec(), size));
    }
}

/// A clock the test sets; the field is the time it reports, in milliseconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct ManualClock(pub u32);

impl Clock for ManualClock {
    fn now_ms(&mut self) -> u32 {
        self.0
    }
}
