//! `play --poke "<f> <directive> <args>..."` and `play --poke-file FILE`:
//! pokes (`d2_sim::poke`) applied on the server thread between frames,
//! through the link's before-pump hook ([`ThreadLink::set_before_pump`]).
//!
//! When a poke runs:
//!
//! - `--poke "<f>..."`: `f` is the absolute server frame (`Game.frame` after
//!   the tick): the poke runs once frame f − 1 has run, before frame f's drain
//!   and tick.
//! - `--poke-file`: the file's ticks are relative:
//!   tick 0 is the first frame after the one in which the local client
//!   reached state 4 (in game), so tick t runs when `Game.frame` = F0 + t,
//!   F0 being that frame (seen by the hook before the next pump).
//!
//! References are resolved by `d2_sim::poke::apply` on the state the
//! hook sees (every reference form; `@wp` from the waypoint
//! table of the game). Each result is printed to stderr. Without either
//! flag nothing is installed.
//!
//! `msg` is not applied to the game: its bytes, references resolved there, go
//! through the host's client sender ([`apply_on_link`]: `Host::send_game` for
//! the local client, with its duplicate filter), and the server handles them in
//! frame f's drain. The C→S layouts (the `d2-proto` tables) live here, not in
//! `d2-sim`: [`check_msg`] at parse time, [`msg_bytes`] after
//! `d2_sim::poke::msg_values` resolved the values.

use d2_proto::schema::{Field, FieldType};
use d2_sim::poke::{self, Directive, GotoTarget, GotoWalk, PokeFile, PokeOp};
use d2_sim::units::lists::client_state;

use super::server_thread::{ThreadLink, ThreadStopped};
use super::single_player::{local_player, Link, Sim};
use crate::bridge::link::LOCAL_CLIENT;

/// When an entry runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum When {
    /// Absolute server frame f: runs when `Game.frame` ≥ f − 1.
    Frame(i32),
    /// Relative tick t of a poke file: runs when `Game.frame` ≥ F0 + t.
    Tick(u32),
}

/// One scheduled poke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub when: When,
    pub op: PokeOp,
}

/// Parses one `--poke` value: `<f> <directive> <args>...` (or `<f> spawn...`).
pub fn parse_poke_arg(s: &str) -> Result<Entry, String> {
    let toks: Vec<&str> = s.split_ascii_whitespace().collect();
    let Some((f, rest)) = toks.split_first() else {
        return Err("--poke \"<frame> <directive> <args>...\"".into());
    };
    let f: i32 = f
        .parse()
        .ok()
        .filter(|&f| f >= 1)
        .ok_or_else(|| format!("--poke: frame {f:?}: a server frame ≥ 1"))?;
    let op = poke::parse_op(rest)
        .and_then(|op| check_msg(&op).map(|()| op))
        .map_err(|e| format!("--poke {s:?}: {e}"))?;
    Ok(Entry {
        when: When::Frame(f),
        op,
    })
}

/// The entries of a poke file.
pub fn parse_poke_file(text: &str) -> Result<Vec<Entry>, String> {
    let f = PokeFile::parse(text).map_err(|e| e.to_string())?;
    for (k, l) in f.lines.iter().enumerate() {
        check_msg(&l.op)
            .map_err(|e| format!("poke line {} (at {} {}): {e}", k + 1, l.tick, l.op))?;
    }
    Ok(f.lines
        .into_iter()
        .map(|l| Entry {
            when: When::Tick(l.tick),
            op: l.op,
        })
        .collect())
}

/// The frame after which `when` is due (`Game.frame` must have reached
/// it), `None` while the anchor F0 is unknown.
pub fn due_after(when: When, anchor: Option<i32>) -> Option<i32> {
    match when {
        When::Frame(f) => Some(f - 1),
        When::Tick(t) => anchor.map(|a| a.saturating_add(i32::try_from(t).unwrap_or(i32::MAX))),
    }
}

/// Splits `state-dump`'s `--poke` entries: every
/// entry due after a tick (absolute frame 2 or later) runs at the tick end
/// (first), the hook's point in the game, so what a directive sends at once (a
/// warp's or a `goto` step's 0x07) leaves in that frame's flush; the
/// others (second: due before the first tick) between frames. A `goto`'s
/// later steps also run at the tick end.
pub fn split_tick_end(entries: Vec<Entry>) -> (Vec<Entry>, Vec<Entry>) {
    entries
        .into_iter()
        .partition(|e| matches!(e.when, When::Frame(f) if f >= 2))
}

/// One poke result as a runner records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Logged {
    /// The server frame after which it ran.
    pub frame: i32,
    /// The directive as written.
    pub op: String,
    /// `ok`, `failed`, `unresolved` or `gap` ([`poke::PokeResult::code`]).
    pub code: &'static str,
    /// The line printed to stderr.
    pub line: String,
    /// Why it failed, when the result says ([`poke::PokeResult::FailedWith`],
    /// [`poke::PokeResult::Gap`]).
    pub note: Option<String>,
}

/// Where a runner collects the poke results, in the order they ran
/// (`repro` puts them in its report).
pub type ResultLog = std::sync::Arc<std::sync::Mutex<Vec<Logged>>>;

/// The pending pokes of a game.
#[derive(Debug, Default)]
pub struct Schedule {
    /// A copy of every result line goes here too, when set.
    log: Option<ResultLog>,
    pending: Vec<Entry>,
    /// `goto` walks still stepping, with their state.
    walking: Vec<(Entry, GotoWalk)>,
    /// F0: the frame in which the local client was first seen in game.
    anchor: Option<i32>,
}

impl Schedule {
    pub fn new(entries: Vec<Entry>) -> Self {
        Self {
            log: None,
            pending: entries,
            walking: Vec::new(),
            anchor: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.walking.is_empty()
    }

    /// The schedule, also logging its result lines to `log`.
    pub fn with_log(mut self, log: ResultLog) -> Self {
        self.log = Some(log);
        self
    }

    /// Runs every due entry in order on the link's game and prints its
    /// result.
    pub fn run_due<C: d2_server::seams::Clock>(&mut self, l: &mut Link<C>) {
        if self.is_empty() {
            return;
        }
        let s = &l.host().game;
        let frame = s.game.frame;
        if self.anchor.is_none()
            && s.game
                .lists
                .client(d2_sim::units::ClientId(LOCAL_CLIENT))
                .is_some_and(|c| c.state == client_state::IN_GAME)
        {
            self.anchor = Some(frame);
        }
        if local_player(s).is_none() {
            return;
        }
        let anchor = self.anchor;
        let (due, later): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|e| due_after(e.when, anchor).is_some_and(|f| frame >= f));
        self.pending = later;
        let walks = std::mem::take(&mut self.walking);
        let due = walks
            .into_iter()
            .map(|(e, w)| (e, Some(w)))
            .chain(due.into_iter().map(|e| (e, None)));
        for (e, walk) in due {
            let r = match (&e.op, walk) {
                (PokeOp::Directive(Directive::Goto(t)), w) => {
                    let (r, w) = goto_now(&mut l.host_mut().game, *t, w.unwrap_or_default());
                    if r == poke::PokeResult::Pending {
                        self.walking.push((e, w));
                        continue;
                    }
                    r
                }
                _ => apply_on_link(l, &e.op),
            };
            print_result(
                frame,
                due_after(e.when, anchor),
                &e.op,
                &r,
                self.log.as_ref(),
            );
        }
    }
}

/// Prints one poke result to stderr (and adds it to `log`, when set);
/// `due` is the frame it was due after (a later `frame` is marked late).
fn print_result(
    frame: i32,
    due: Option<i32>,
    op: &PokeOp,
    r: &poke::PokeResult,
    log: Option<&ResultLog>,
) {
    let late = match due {
        Some(f) if frame > f => format!(" (late: due after frame {f})"),
        _ => String::new(),
    };
    let line = format!(
        "poke: after frame {frame}{late}: {op}: {}{}",
        r.code(),
        match r {
            poke::PokeResult::Ok(Some(g)) => format!(" guid {g}"),
            poke::PokeResult::Unresolved(u) => format!(" {u}"),
            poke::PokeResult::Gap(why) | poke::PokeResult::FailedWith(why) => format!(" ({why})"),
            _ => String::new(),
        }
    );
    eprintln!("{line}");
    if let Some(Ok(mut l)) = log.map(|l| l.lock()) {
        l.push(Logged {
            frame,
            op: op.to_string(),
            code: r.code(),
            line,
            note: match r {
                poke::PokeResult::Gap(why) | poke::PokeResult::FailedWith(why) => Some(why.clone()),
                _ => None,
            },
        });
    }
}

/// The `objects` rows with operate function 23 (`@wp`).
pub(crate) use d2_server::dev::waypoint_classes;

/// Runs `op` on `s` now: the local player is `@player`,
/// `@wp` and `item` use the game's own tables. No local player yet:
/// `unresolved @player`.
pub fn apply_now(s: &mut Sim, op: &PokeOp) -> poke::PokeResult {
    let Some((player, _)) = local_player(s) else {
        return poke::PokeResult::Unresolved("@player".into());
    };
    d2_server::dev::apply_now(s, player, op)
}

/// One step of a `goto` walk on `s`, with the walk's
/// state; no local player: `unresolved @player`.
pub fn goto_now(s: &mut Sim, t: GotoTarget, mut walk: GotoWalk) -> (poke::PokeResult, GotoWalk) {
    let Some((player, _)) = local_player(s) else {
        return (poke::PokeResult::Unresolved("@player".into()), walk);
    };
    let env = poke::Env::new(player);
    let r = poke::goto_step(&mut s.game, &mut s.events, &env, &t, &mut walk);
    // As [`apply_now`]: a step's warp follows its pets now.
    s.world.handler_work(&mut s.game, &mut s.events);
    (r, walk)
}

/// Runs `op` on the link's server game now: `msg` through the host's
/// client sender, every other directive by
/// [`apply_now`]. `msg` results: `ok` when the sender passed the bytes
/// on, `failed` "duplicate filter" when its filter dropped them,
/// `failed` with the error when the sender refused them.
pub fn apply_on_link<C: d2_server::seams::Clock>(l: &mut Link<C>, op: &PokeOp) -> poke::PokeResult {
    apply_on_host(l.host_mut(), op)
}

/// The host of the app's link.
pub type ServerHost<C> = d2_server::host::Host<
    Sim,
    d2_server::adapters::ProtoSizes,
    crate::bridge::local::PendingSession,
    C,
>;

/// The first byte of a queue-walk mark (`d2_sim::wiring::action`).
fn is_walk_mark(b: u8) -> bool {
    use d2_sim::wiring::action::reaction::LEVEL_UP_ITEMS_MARK;
    use d2_sim::wiring::action::{
        GROUND_ITEM_MARK, ITEM_BREAK_MARK, JOIN_ITEM_MARK, MERC_ITEMS_MARK, PLAYER_ITEMS_MARK,
        PLAYER_SOUND_MARK, STORE_ITEMS_MARK,
    };
    matches!(
        b,
        GROUND_ITEM_MARK
            | ITEM_BREAK_MARK
            | LEVEL_UP_ITEMS_MARK
            | d2_sim::wiring::action::pending::SOCKET_RESTORE_MARK
            | d2_sim::wiring::action::pending::KEY_USE_MARK
            | PLAYER_ITEMS_MARK
            | PLAYER_SOUND_MARK
            | STORE_ITEMS_MARK
            | MERC_ITEMS_MARK
            | JOIN_ITEM_MARK
    )
}

/// Queues what the sim sent during a poke run at the tick end into the clients'
/// buffers now, so it leaves in this frame's flush as in the game (a `goto` step's
/// 0x07); left in the sim, the next tick would queue it. A queueing failure is
/// printed.
pub fn queue_sent_now<C: d2_server::seams::Clock>(h: &mut ServerHost<C>) {
    use d2_server::adapters::handlers::world::WorldHost;
    let sent = h.game.world.take_sent(&mut h.game.events);
    // The queue walk's host-only marks (ground item, player items / sound)
    // are the tick's to turn into the real messages (`SimGame` tick,
    // `take_sent`): left queued, not sent raw.
    let (marks, sent): (Vec<_>, Vec<_>) = sent.into_iter().partition(|(_, b)| {
        use d2_sim::wiring::action::reaction::LEVEL_UP_ITEMS_MARK;
        use d2_sim::wiring::action::{
            GROUND_ITEM_MARK, ITEM_BREAK_MARK, JOIN_ITEM_MARK, MERC_ITEMS_MARK, PLAYER_ITEMS_MARK,
            PLAYER_SOUND_MARK, STORE_ITEMS_MARK,
        };
        b.len() == 5
            && matches!(
                b[0],
                GROUND_ITEM_MARK
                    | ITEM_BREAK_MARK
                    | LEVEL_UP_ITEMS_MARK
                    | d2_sim::wiring::action::pending::SOCKET_RESTORE_MARK
                    | d2_sim::wiring::action::pending::KEY_USE_MARK
                    | PLAYER_ITEMS_MARK
                    | PLAYER_SOUND_MARK
                    | STORE_ITEMS_MARK
                    | MERC_ITEMS_MARK
                    | JOIN_ITEM_MARK
            )
    });
    h.game.world.rest.sent.splice(0..0, marks);
    for (unit, bytes) in sent {
        // A queue-walk mark (a ground item's, a player's items or sound) is
        // answered by the tick's client pass, not here: nothing at the tick
        // end stands for it.
        if bytes.len() == 5 && is_walk_mark(bytes[0]) {
            continue;
        }
        // A player without a client receives nothing.
        if let Some(c) = h.game.client_of(unit) {
            if let Err(e) = h.queue_now(c, &bytes) {
                eprintln!("poke: queueing a message at the tick end: {e}");
            }
        }
    }
}

/// [`apply_on_link`] on the link's host: the form a tick-end hook
/// (`LocalLink::set_tick_end`) calls.
pub fn apply_on_host<C: d2_server::seams::Clock>(
    h: &mut ServerHost<C>,
    op: &PokeOp,
) -> poke::PokeResult {
    if let PokeOp::Directive(d @ (Directive::Operate { .. } | Directive::Talk { .. })) = op {
        return interact_on_host(h, d);
    }
    let PokeOp::Directive(poke::Directive::Msg { id, args }) = op else {
        return apply_now(&mut h.game, op);
    };
    let bytes = {
        let s = &h.game;
        let Some((player, _)) = local_player(s) else {
            return poke::PokeResult::Unresolved("@player".into());
        };
        let waypoints = waypoint_classes(s);
        let env = poke::Env {
            player,
            waypoint_classes: &waypoints,
            items: Some(&s.world.tables),
        };
        match poke::msg_values(&s.game, &s.events, &env, args)
            .and_then(|v| msg_bytes(*id, args, &v))
        {
            Ok(b) => b,
            Err(reference) => return poke::PokeResult::Unresolved(reference),
        }
    };
    match h.send_game(LOCAL_CLIENT, &bytes) {
        Ok(Some(_)) => poke::PokeResult::Ok(None),
        Ok(None) => poke::PokeResult::FailedWith("duplicate filter".into()),
        Err(e) => poke::PokeResult::FailedWith(e.to_string()),
    }
}

/// `operate` / `talk`: each handler call's
/// bytes (the `msg` encoder, layouts of `sim/client-messages.tsv`) run by
/// the server's dispatcher now (`Host::dispatch_now`), in order. `ok`
/// with the target's GUID when every call returned 0; else `failed`
/// naming the first id whose result was not 0, and no later call runs.
pub fn interact_on_host<C: d2_server::seams::Clock>(
    h: &mut ServerHost<C>,
    d: &Directive,
) -> poke::PokeResult {
    let (guid, calls) = {
        let s = &h.game;
        let Some((player, _)) = local_player(s) else {
            return poke::PokeResult::Unresolved("@player".into());
        };
        let waypoints = waypoint_classes(s);
        let env = poke::Env {
            player,
            waypoint_classes: &waypoints,
            items: Some(&s.world.tables),
        };
        match poke::interact_calls(&s.game, &s.events, &env, d) {
            Some(Ok(c)) => c,
            Some(Err(reference)) => return poke::PokeResult::Unresolved(reference),
            None => unreachable!("operate / talk only"),
        }
    };
    for c in &calls {
        let bytes = match encode_msg(c.id, &c.values) {
            Ok(b) => b,
            Err(e) => return poke::PokeResult::FailedWith(e),
        };
        match h.dispatch_now(LOCAL_CLIENT, &bytes) {
            Some(d2_server::seams::ResultCode::Done) => {}
            Some(code) => {
                return poke::PokeResult::FailedWith(format!(
                    "{:#04x} returned {}",
                    c.id, code as u8
                ))
            }
            None => return poke::PokeResult::FailedWith("no player to dispatch for".into()),
        }
    }
    poke::PokeResult::Ok(Some(guid))
}

// ---- msg: C→S layouts ------

/// Largest value a `msg` field holds.
fn field_max(ty: FieldType) -> u32 {
    match ty {
        FieldType::U8 => 0xFF,
        FieldType::U16 => 0xFFFF,
        FieldType::Bits(n) if n < 32 => (1u32 << n) - 1,
        FieldType::Bit(_) => 1,
        _ => u32::MAX,
    }
}

/// Name, size and fields of C→S message `id` when `msg` can write it:
/// an id in 0x01..=0x70 with a fixed size whose layout has only integer
/// fields (`u8`, `u16`, `u32`, `uN`, `bitN`); bytes the layout does not list
/// are written 0.
pub fn msg_layout(id: u8) -> Result<(&'static str, usize, &'static [Field<'static>]), String> {
    let m = d2_proto::transport::client_message(id)
        .filter(|_| (0x01..=0x70).contains(&id))
        .ok_or_else(|| format!("msg id {id:#04x}: a C→S id 0x01..0x70"))?;
    let Some(size) = m.transport_size.fixed() else {
        return Err(format!("msg {id:#04x} ({}) has no fixed size", m.name));
    };
    for f in m.layout {
        let off = usize::from(f.offset.unwrap_or(0));
        let bytes = match f.ty {
            FieldType::U8 => off..off + 1,
            FieldType::U16 => off..off + 2,
            FieldType::U32 => off..off + 4,
            FieldType::Bits(n) => off..off + usize::from(n).div_ceil(8),
            FieldType::Bit(b) => off + usize::from(b) / 8..off + usize::from(b) / 8 + 1,
            _ => {
                return Err(format!(
                    "msg {id:#04x} ({}): field {} is not an integer",
                    m.name, f.name
                ))
            }
        };
        if f.offset.is_none() || bytes.end > size {
            return Err(format!(
                "msg {id:#04x} ({}): field {} has no offset in the message",
                m.name, f.name
            ));
        }
    }
    Ok((m.name, size, m.layout))
}

/// The bytes of C→S message `id` with `values` in its fields:
/// the id, then each field little-endian at its offset (`uN` / `bitN`
/// into the u32 at the offset); `transport_size` bytes.
pub fn encode_msg(id: u8, values: &[u32]) -> Result<Vec<u8>, String> {
    let (name, size, fields) = msg_layout(id)?;
    if values.len() != fields.len() {
        return Err(format!(
            "msg {id:#04x} ({name}) takes {} value(s), got {}",
            fields.len(),
            values.len()
        ));
    }
    let mut out = vec![0u8; size];
    out[0] = id;
    for (f, &v) in fields.iter().zip(values) {
        if v > field_max(f.ty) {
            return Err(format!("{v} does not fit field {}", f.name));
        }
        let off = usize::from(f.offset.unwrap_or(0));
        match f.ty {
            FieldType::U8 => out[off] = v as u8,
            FieldType::U16 => out[off..off + 2].copy_from_slice(&(v as u16).to_le_bytes()),
            FieldType::U32 => out[off..off + 4].copy_from_slice(&v.to_le_bytes()),
            FieldType::Bits(n) | FieldType::Bit(n) => {
                let shift = if matches!(f.ty, FieldType::Bit(_)) {
                    u32::from(n)
                } else {
                    0
                };
                let word = v << shift;
                // Only the bytes the field's bits reach (a `u15` / `bit15`
                // pair shares a u16 with the field after it).
                for (k, b) in word.to_le_bytes().into_iter().enumerate() {
                    if b != 0 {
                        out[off + k] |= b;
                    }
                }
            }
            _ => unreachable!("msg_layout admits integer fields only"),
        }
    }
    Ok(out)
}

/// Checks a `msg` directive against its id's layout:
/// an id `msg` can write, one value per field, and
/// every number fits its field. Other operations pass.
pub fn check_msg(op: &PokeOp) -> Result<(), String> {
    let PokeOp::Directive(poke::Directive::Msg { id, args }) = op else {
        return Ok(());
    };
    let (name, _, fields) = msg_layout(*id)?;
    if args.len() != fields.len() {
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        return Err(format!(
            "`msg {id:#04x}` ({name}) takes {} value(s) ({}), got {}",
            fields.len(),
            names.join(" "),
            args.len()
        ));
    }
    for (a, f) in args.iter().zip(fields) {
        if let poke::MsgArg::Num(v) = *a {
            if v > field_max(f.ty) {
                return Err(format!(
                    "msg field {}: {v} does not fit (largest {})",
                    f.name,
                    field_max(f.ty)
                ));
            }
        }
    }
    Ok(())
}

/// The bytes of `msg <id>` with its resolved `values` (`args` as
/// written, for the note); `Err`: a value that does not fit its field.
pub fn msg_bytes(id: u8, args: &[poke::MsgArg], values: &[i64]) -> Result<Vec<u8>, String> {
    let (_, _, fields) = msg_layout(id)?;
    let mut out = Vec::with_capacity(values.len());
    for ((a, f), &v) in args.iter().zip(fields).zip(values) {
        match u32::try_from(v).ok().filter(|&v| v <= field_max(f.ty)) {
            Some(v) => out.push(v),
            None => return Err(format!("{a} = {v} does not fit field {}", f.name)),
        }
    }
    encode_msg(id, &out)
}

/// The keyword a `poke` record names (`d`): the directive's, or `spawn`.
pub fn op_keyword(op: &PokeOp) -> &'static str {
    match op {
        PokeOp::Directive(d) => d.keyword(),
        PokeOp::Spawn(_) => "spawn",
    }
}

/// The `poke` record of an absolute `--poke` result (the fields of the poke
/// record): `f` the frame the poke precedes, `frame` = f − 1, `i` its
/// index among the pokes run at that point, `d`, `r`, `guid` when `ok` created
/// a unit, `note` for `unresolved` / `gap`, `src` the canonical directive.
pub fn record_line(f: i32, i: usize, op: &PokeOp, r: &poke::PokeResult) -> String {
    record_line_steps(f, i, op, r, None)
}

/// [`record_line`] with a `goto`'s `steps`.
pub fn record_line_steps(
    f: i32,
    i: usize,
    op: &PokeOp,
    r: &poke::PokeResult,
    steps: Option<u32>,
) -> String {
    use d2_sim::debug::state::json_string;
    let mut o = format!(
        "{{\"k\":\"poke\",\"f\":{f},\"frame\":{},\"i\":{i},\"d\":{},\"r\":{}",
        f - 1,
        json_string(op_keyword(op)),
        json_string(r.code())
    );
    match r {
        poke::PokeResult::Ok(Some(g)) => o.push_str(&format!(",\"guid\":{g}")),
        poke::PokeResult::Unresolved(n)
        | poke::PokeResult::Gap(n)
        | poke::PokeResult::FailedWith(n) => {
            o.push_str(&format!(",\"note\":{}", json_string(n)));
        }
        _ => {}
    }
    if let Some(n) = steps {
        o.push_str(&format!(",\"steps\":{n}"));
    }
    o.push_str(&format!(",\"src\":{}}}", json_string(&op.to_string())));
    o
}

/// The server thread runs the poke between two frames.
impl<C: d2_server::seams::Clock + Send + 'static> crate::bridge::poke::PokeTarget
    for ThreadLink<Link<C>>
{
    type Error = ThreadStopped;
    fn poke(&mut self, op: &PokeOp) -> Result<poke::PokeResult, Self::Error> {
        let op = op.clone();
        self.with(move |l| apply_on_link(l, &op))
    }
    fn goto_step(
        &mut self,
        target: GotoTarget,
        walk: GotoWalk,
    ) -> Result<(poke::PokeResult, GotoWalk), Self::Error> {
        self.with(move |l| goto_now(&mut l.host_mut().game, target, walk))
    }
}

/// Installs `entries` (every one due after a tick, [`split_tick_end`])
/// as the server thread's tick-end hook, as
/// `state-dump` runs them: in every pump that ticks, after the tick and
/// before its flush, `goto` walks stepping first; what a directive sends
/// leaves in that frame's flush ([`queue_sent_now`]). No-op when empty.
pub fn install_tick_end<C: d2_server::seams::Clock + Send + 'static>(
    link: &mut ThreadLink<Link<C>>,
    entries: Vec<Entry>,
) -> Result<(), ThreadStopped> {
    install_tick_end_logged(link, entries, None)
}

/// [`install_tick_end`], also logging the result lines to `log`.
pub fn install_tick_end_logged<C: d2_server::seams::Clock + Send + 'static>(
    link: &mut ThreadLink<Link<C>>,
    entries: Vec<Entry>,
    log: Option<ResultLog>,
) -> Result<(), ThreadStopped> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut entries = entries;
    let mut walking: Vec<(Entry, GotoWalk)> = Vec::new();
    link.with(move |l| {
        l.set_tick_end(Box::new(move |h| {
            let frame = h.game.game.frame;
            let (due, later): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut entries)
                .into_iter()
                .partition(|e| due_after(e.when, None).is_some_and(|f| frame >= f));
            entries = later;
            if due.is_empty() && walking.is_empty() {
                return;
            }
            let due = std::mem::take(&mut walking)
                .into_iter()
                .map(|(e, w)| (e, Some(w)))
                .chain(due.into_iter().map(|e| (e, None)));
            for (e, walk) in due {
                let r = match (&e.op, walk) {
                    (PokeOp::Directive(Directive::Goto(t)), w) => {
                        let (r, w) = goto_now(&mut h.game, *t, w.unwrap_or_default());
                        if r == poke::PokeResult::Pending {
                            walking.push((e, w));
                            continue;
                        }
                        r
                    }
                    _ => apply_on_host(h, &e.op),
                };
                print_result(frame, due_after(e.when, None), &e.op, &r, log.as_ref());
            }
            queue_sent_now(h);
        }))
    })
}

/// Installs `entries` on the game's server thread (no-op when empty).
pub fn install<C: d2_server::seams::Clock + Send + 'static>(
    link: &mut ThreadLink<Link<C>>,
    entries: Vec<Entry>,
) -> Result<(), ThreadStopped> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut schedule = Schedule::new(entries);
    link.set_before_pump(move |l: &mut Link<C>| schedule.run_due(l))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poke_args_and_files_parse_and_schedule() {
        let e = parse_poke_arg("40 time 2 0").unwrap();
        assert_eq!(e.when, When::Frame(40));
        assert_eq!(e.op.to_string(), "time 2 0");
        let e = parse_poke_arg("3 spawn 19 @x+4 @y normal").unwrap();
        assert!(matches!(e.op, PokeOp::Spawn(_)));
        assert!(parse_poke_arg("0 time 2 0").is_err());
        assert!(parse_poke_arg("x time 2 0").is_err());
        assert!(parse_poke_arg("5 time 9 0")
            .unwrap_err()
            .contains("period 9"));
        assert!(parse_poke_arg("").is_err());
        let e = parse_poke_arg("4 msg 0x01 @x+5 @y").unwrap();
        assert_eq!(e.op.to_string(), "msg 1 @x+5 @y");
        assert!(parse_poke_arg("4 msg 0x01 5")
            .unwrap_err()
            .contains("takes 2"));
        let f = parse_poke_file("poke 1\nat 0 seed-game 1 2\nat 7 freeze 1\n").unwrap();
        assert_eq!(f[1].when, When::Tick(7));
        assert!(parse_poke_file("poke 2\n").is_err());
        // Absolute frame f: after frame f − 1; relative tick t: after F0 + t.
        assert_eq!(due_after(When::Frame(40), None), Some(39));
        assert_eq!(due_after(When::Tick(7), None), None);
        assert_eq!(due_after(When::Tick(7), Some(100)), Some(107));
        assert!(Schedule::new(Vec::new()).is_empty());
    }

    #[test]
    fn msg_takes_the_fixed_ids_whose_layout_lists_every_byte() {
        let ok: Vec<u8> = (0x01..=0x70).filter(|&id| msg_layout(id).is_ok()).collect();
        // Walk / run / skills 0x01-0x13 (0x0B, 0x12 without fields), and
        // e.g. skill select 0x3C, hotkey 0x51, swap weapons 0x60.
        for id in (0x01..=0x13).chain([0x3C, 0x51, 0x60]) {
            assert!(ok.contains(&id), "{id:#04x}");
        }
        // Unlisted bytes are written 0 (0x49: bytes 7-8; 0x1A: bytes 6-8).
        for id in [0x1A, 0x49] {
            assert!(ok.contains(&id), "{id:#04x}");
        }
        // Chat (no fixed size), never-valid ids, variable sizes.
        for id in [0x14, 0x15, 0x2B, 0x66, 0x67, 0x68, 0x6C] {
            assert!(!ok.contains(&id), "{id:#04x}");
        }
        for &id in &ok {
            let (_, size, fields) = msg_layout(id).unwrap();
            let zeros = encode_msg(id, &vec![0; fields.len()]).unwrap();
            assert_eq!(zeros.len(), size, "{id:#04x}");
            assert_eq!(zeros[0], id);
            assert!(zeros[1..].iter().all(|&b| b == 0));
        }
        assert_eq!(
            ok.iter()
                .map(|id| format!("{id:02X}"))
                .collect::<Vec<_>>()
                .join(" "),
            MSG_IDS
        );
    }

    #[test]
    fn pokes_due_after_a_tick_run_at_the_tick_end() {
        let e = |s: &str| parse_poke_arg(s).unwrap();
        let (end, rest) = split_tick_end(vec![
            e("1 pos @player 5 6"),
            e("4 pos @player 1 2"),
            e("4 warp 2"),
            e("5 goto unit 148"),
            e("5 talk @1:148"),
            e("9 operate @2:267"),
            e("12 time 2 0"),
        ]);
        let text = |v: &[Entry]| v.iter().map(|e| e.op.to_string()).collect::<Vec<_>>();
        assert_eq!(
            text(&end),
            [
                "pos @player 1 2",
                "warp 2",
                "goto unit 1:148",
                "talk @1:148",
                "operate @2:267",
                "time 2 0"
            ]
        );
        assert_eq!(text(&rest), ["pos @player 5 6"]);
    }

    #[test]
    fn operate_and_talk_bytes_are_the_layouts_of_their_ids() {
        let d = poke::parse_directive_text("talk @1:148 trade hire quest:92 close").unwrap();
        let bytes: Vec<String> = poke::interact_calls_with(&d, 1, 12, 1)
            .iter()
            .map(|c| {
                encode_msg(c.id, &c.values)
                    .unwrap()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            })
            .collect();
        assert_eq!(
            bytes,
            [
                "13010000000c000000",
                "2f000000000c000000",
                "38010000000c00000000000000",
                "38030000000c00000001000000",
                "310c0000005c000000",
                "30000000000c000000",
            ]
        );
    }

    /// The ids `msg` accepts, as `poke.py --selftest` checks.
    const MSG_IDS: &str = "01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 16 17 18 19 1A 1B 1C 1D 1E 1F 20 21 22 23 24 25 26 27 28 29 2A 2D 2E 2F 30 31 32 33 34 35 36 37 38 39 3A 3B 3C 3D 3E 3F 40 41 42 43 44 45 46 47 48 49 4B 4C 4D 4F 50 51 52 53 54 58 59 5D 5E 5F 60 61 62 63 69 6A 6B 6D 6E 70";

    #[test]
    fn msg_bytes_are_the_id_then_the_fields_little_endian() {
        assert_eq!(
            encode_msg(0x01, &[0x1234, 0x5678]).unwrap(),
            [0x01, 0x34, 0x12, 0x78, 0x56]
        );
        assert_eq!(
            encode_msg(0x06, &[1, 0xAABBCCDD]).unwrap(),
            [0x06, 1, 0, 0, 0, 0xDD, 0xCC, 0xBB, 0xAA]
        );
        // 0x3C: skill bits 0-30 and the left flag bit 31 of the u32 at 1.
        assert_eq!(
            encode_msg(0x3C, &[36, 1, u32::MAX]).unwrap(),
            [0x3C, 36, 0, 0, 0x80, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        // 0x51: skill bits 0-14 and bit 15 of the u16 at 1, slot at 3.
        assert_eq!(
            encode_msg(0x51, &[5, 1, 3, 7]).unwrap(),
            [0x51, 5, 0x80, 3, 0, 7, 0, 0, 0]
        );
        assert_eq!(encode_msg(0x60, &[]).unwrap(), [0x60]);
        assert!(encode_msg(0x01, &[1]).unwrap_err().contains("takes 2"));
        assert!(encode_msg(0x01, &[0x10000, 1])
            .unwrap_err()
            .contains("does not fit field x"));
    }

    #[test]
    fn msg_lines_are_checked_against_the_layout() {
        for (arg, needle) in [
            ("4 msg 0x14 1 2 3 4", "no fixed size"),
            ("4 msg 0x1A 1", "takes 2 value(s)"),
            ("4 msg 0x01 1", "takes 2 value(s) (x y), got 1"),
            ("4 msg 0x01 1 2 3", "got 3"),
            ("4 msg 0x01 65536 1", "does not fit"),
            ("4 msg 0x3C 1 2 3", "field left: 2 does not fit"),
        ] {
            let e = parse_poke_arg(arg).unwrap_err();
            assert!(e.contains(needle), "{arg:?}: {e}");
        }
        assert!(parse_poke_file("poke 1\nat 0 msg 0x01 1\n")
            .unwrap_err()
            .contains("takes 2"));
        // Bytes the layout does not list are 0 (0x49: bytes 7-8).
        assert_eq!(
            encode_msg(0x49, &[0x1234_5678, 3]).unwrap(),
            [0x49, 0x78, 0x56, 0x34, 0x12, 3, 0, 0, 0]
        );
        let e = parse_poke_arg("4 msg 0x01 @x+2 @y").unwrap();
        assert_eq!(e.op.to_string(), "msg 1 @x+2 @y");
        let PokeOp::Directive(poke::Directive::Msg { id, args }) = &e.op else {
            panic!("{:?}", e.op);
        };
        // Resolved values (player at (5000, 4000)) to bytes; one that
        // does not fit names the reference.
        assert_eq!(
            msg_bytes(*id, args, &[5002, 4000]).unwrap(),
            [0x01, 0x8A, 0x13, 0xA0, 0x0F]
        );
        assert!(msg_bytes(*id, args, &[-1, 4000])
            .unwrap_err()
            .contains("@x+2 = -1 does not fit field x"));
    }
}
