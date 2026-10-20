//! The narrow interfaces `d2-server` needs from other crates, defined here
//! so the transport and host loop can be built and tested before those
//! crates exist. Each trait names its provider; the adapters in
//! [`crate::adapters`] implement them on the real crates. [`crate::host`]
//! and [`crate::dispatch`] are generic over them.
//!
//! | Trait | Provider |
//! |---|---|
//! | [`MessageSizes`] | `d2-proto` (size rules from `sim/*-messages.tsv`): [`crate::adapters::ProtoSizes`] |
//! | [`Intents`] | `d2-sim` (player gate state, intent handlers, client list): [`crate::adapters::SimGame`] |
//! | [`Tick`] | `d2-sim::tick` (one game tick): [`crate::adapters::SimGame`] |
//! | [`SessionHandler`] | `d2-server` session code |
//! | [`Clock`] | host: [`crate::host::SystemClock`]; tests: a manual clock |

/// Client id as the transport carries it (u32; 0 for the local client).
pub type ClientId = u32;

/// Why a size rule gives no size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeError {
    /// Table entry 0, or an id outside the table: never a valid message.
    Invalid,
    /// Fewer bytes than the rule needs, or a string without its NUL.
    Incomplete,
    /// The C→S chat rule came out negative (signed byte). What the
    /// transport classifier should do with it is undecided; callers must
    /// not treat it as either of the other results.
    Negative(i32),
}

/// Per-direction message size rules.
///
/// Provider: `d2-proto`, generated from the `transport_size` column of
/// `tables/sim/client-messages.tsv` and the `size` column of
/// `tables/sim/server-messages.tsv`.
pub trait MessageSizes {
    /// C→S size rule for ids 0x00..=0x70 (`msg[0]` is the id,
    /// `msg` is non-empty). Ids ≥ 0x71 never reach it: the classifier
    /// handles them (0x71..=0xFE invalid, 0xFF fixed at 16 bytes).
    fn client_size(&self, msg: &[u8]) -> Result<usize, SizeError>;

    /// S→C size rule (`msg` non-empty); ids past the generated table, and
    /// the reserved envelope id, are [`SizeError::Invalid`].
    fn server_size(&self, msg: &[u8]) -> Result<usize, SizeError>;
}

/// Dispatch result code. The host ignores it;
/// D2RUST keeps it as a diagnostic only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ResultCode {
    /// 0: done (also "gate refused" and the stub0 handlers).
    Done = 0,
    /// 1: refused (target missing, out of range, wrong state).
    Refused = 1,
    /// 2: invalid field (bad unit type, wrong act, out of range index).
    Invalid = 2,
    /// 3: malformed (wrong size, stub, bad id).
    Malformed = 3,
}

/// The player of a client as the gate sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerLookup {
    /// The client is in no game (the game lookup fails): dropped and logged.
    NotInGame,
    /// In a game, but no player unit (or the unit is not unit type 0):
    /// dropped.
    NoPlayer,
    /// A player unit.
    Player(PlayerGate),
}

/// The player fields the dispatch gate reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerGate {
    /// Unit mode: 0 = death, 0x11 = dead.
    pub mode: u32,
    /// Has state 0x36 (54, uninterruptable).
    pub uninterruptable: bool,
}

/// Subtile position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pos {
    /// Subtile x.
    pub x: i32,
    /// Subtile y.
    pub y: i32,
}

/// What the point-message parser reads from the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointState {
    /// Player position (dynamic path, or static path for unit types 2, 4,
    /// 5).
    pub player: Pos,
    /// Frame of the last accepted point target.
    pub last_accept: i32,
}

/// Result of the unit-target lookup before the range test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitTarget {
    /// No such unit → 1.
    Missing,
    /// An item the player owns → accept without a range test.
    OwnedItem,
    /// Target in another act than the player → 2.
    OtherAct,
    /// Both positions, for the 50-subtile test (→ 1 when out of range).
    At { player: Pos, target: Pos },
}

/// Receives the server's messages to clients (D2MOO
/// `D2GAME_PACKETS_SendPacket`). Implemented by
/// [`crate::buffers::ClientBuffers`].
pub trait MessageSink {
    /// Queue `msg` for `client`. An unknown client is ignored.
    fn queue(&mut self, client: ClientId, msg: &[u8]) -> Result<(), crate::buffers::QueueError>;

    /// The client has a queued buffer (read by
    /// the vitals sync's force rule).
    /// Default: no.
    fn has_queued(&self, _client: ClientId) -> bool {
        false
    }

    /// A direct send: straight to the client's receive
    /// lists, ahead of anything still buffered. Default: refused (a sink
    /// with no receive lists cannot do it; never a silent queue).
    fn send_direct(
        &mut self,
        _client: ClientId,
        msg: &[u8],
    ) -> Result<(), crate::buffers::QueueError> {
        Err(crate::buffers::QueueError::NoDirect(
            msg.first().copied().unwrap_or(0),
        ))
    }

    /// Flushes one client's buffers into its receive lists
    /// (used by the leave of a game). Default: refused.
    fn flush_client(&mut self, _client: ClientId) -> Result<(), crate::buffers::QueueError> {
        Err(crate::buffers::QueueError::NoDirect(0))
    }
}

/// Game state the dispatcher reads and the intent handlers.
///
/// Provider: `d2-sim` (player units, player data, the game's client list
/// in list order, and one handler per C→S id).
pub trait Intents {
    /// The client's game and player.
    fn player(&self, client: ClientId) -> PlayerLookup;

    /// The game frame counter.
    fn frame(&self) -> i32;

    /// Before the point / unit parse of message
    /// `msg`: the provider brings the facts the parse reads (the
    /// client's player and, for a unit message, the target) up to date.
    /// Default: nothing (the facts are staged by the caller).
    fn refresh_targets(&mut self, client: ClientId, msg: &[u8]) {
        let _ = (client, msg);
    }

    /// Point parser inputs; `None` when the player has
    /// no player data (→ 2).
    fn point_state(&self, client: ClientId) -> Option<PointState>;

    /// Records an accepted point target: the last accepted frame := `frame`.
    fn set_point_accept(&mut self, client: ClientId, frame: i32);

    /// Queues S→C 0x15 (reassign player) to resync `client`.
    fn queue_resync(&mut self, client: ClientId, out: &mut dyn MessageSink);

    /// Unit-target lookup for unit messages, called
    /// with a unit type already checked < 6.
    fn unit_target(&self, client: ClientId, unit_type: u32, unit_id: u32) -> UnitTarget;

    /// The intent handler for an id with kind `handler` (the
    /// `client-messages.tsv` `kind` column), called after the gate, the
    /// size check and the server-side parse.
    /// `msg` is the drained copy (at most 0x1FC bytes);
    /// `size` is the full size. Point and unit messages arrive already
    /// range-checked; skill messages still owe the `pierce_idx`
    /// increment.
    fn handle(
        &mut self,
        client: ClientId,
        msg: &[u8],
        size: usize,
        out: &mut dyn MessageSink,
    ) -> ResultCode;

    /// The game's client list in list order: the flush order.
    fn clients(&self) -> Vec<ClientId>;

    /// A client is gone: its connection ended or was dropped, with or
    /// without a leave message. The game takes it out (its player leaves
    /// as with a leave); `out` reaches the other clients. Default: the
    /// game keeps nothing of clients, so nothing happens.
    fn client_gone(&mut self, client: ClientId, out: &mut dyn MessageSink) {
        let _ = (client, out);
    }

    /// The game's part of a drained queue-0 message (the single-player
    /// session sequence C→S 0x67 → 0x6B). True when the
    /// game handled it; false hands it to the host's [`SessionHandler`].
    /// Default: the game handles none.
    fn session_message(
        &mut self,
        client: ClientId,
        msg: &[u8],
        size: usize,
        out: &mut dyn MessageSink,
    ) -> bool {
        let _ = (client, msg, size, out);
        false
    }

    /// A mod message from `client` (`docs/plans/mods/seams.md` MS13): the
    /// payload of a C→S envelope (`d2_proto::envelope`), drained from the
    /// game queue in order with the client's game messages. Only a mod
    /// game's wire admits envelopes. The game's mod messages to clients go
    /// to `out` as S→C envelopes. True when the game took it; false drops
    /// it. Default: the game takes none.
    fn mod_message(&mut self, client: ClientId, payload: &[u8], out: &mut dyn MessageSink) -> bool {
        let _ = (client, payload, out);
        false
    }

    /// The host's millisecond clock of this frame, read once per frame
    /// before the drain (`Host::frame`): the game's tick-count input
    /// (`d2-sim` never reads a clock itself). Default: the game has no
    /// reader.
    fn set_host_tick(&mut self, ms: u32) {
        let _ = ms;
    }
}

/// One game tick: frame += 1, then the steps.
///
/// Provider: `d2-sim::tick`.
pub trait Tick {
    /// Runs one tick; events go to `out` in production order.
    fn tick(&mut self, out: &mut dyn MessageSink);

    /// The rate the game's ticks run at: the host schedules them at this
    /// rate ([`crate::host::TickDriver`]). No default, so every game states
    /// its rate; test fakes say [`d2_sim::tick::TickRate::BASE`] on purpose.
    fn tick_rate(&self) -> d2_sim::tick::TickRate;

    /// The game's part of a flush that sends: the per-client vitals sync,
    /// queued before the client's buffers are sent.
    /// Default: nothing.
    fn flush_sync(&mut self, out: &mut dyn MessageSink) {
        let _ = out;
    }
}

/// System messages 0x67..=0x70. Provider: `d2-server`'s
/// session code (game creation, join, leave, save upload).
pub trait SessionHandler {
    /// One drained queue-0 message (`msg` is the truncated drain copy).
    fn system_message(
        &mut self,
        client: ClientId,
        msg: &[u8],
        size: usize,
        out: &mut dyn MessageSink,
    );
}

/// Added to the host clock for the game's tick-count input (ms). The
/// game's host clock is the machine's uptime: the absolute host-clock
/// tests (portal 5000 ms, waypoint 10000 ms, door 500 ms) compare it with
/// a player's stored hostile time, 0 for a fresh player, so on any real
/// host they pass from the first frame (scenario `sys-town-portal-tome`
/// enters a tome
/// portal at frame 70, 2.8 s after the join). D2RUST's own: one hour of
/// uptime, added only where the host hands the tick to the game
/// (`Host::frame_with`), so the tick driver, the drain and the client's
/// clock are unchanged.
pub const HOST_CLOCK_ORIGIN_MS: u32 = 3_600_000;

/// Millisecond wall clock (host only). Injected so tests are deterministic.
/// The host extends its readings to the 1 ms base clock
/// ([`crate::host::BaseClock`]) that ticks are scheduled on.
pub trait Clock {
    /// Milliseconds since an arbitrary origin, wrapping at 2^32.
    fn now_ms(&mut self) -> u32;
}
