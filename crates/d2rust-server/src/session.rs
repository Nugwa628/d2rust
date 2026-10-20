//! Online sessions: one per connection's control stream, from its
//! handshake to its logout or loss (contracts section 6, "Sessions,
//! dispatch and the lobby seam").
//!
//! - [`SessionCtx`]: what the server knows about a session: who it is
//!   logged in as, the character it selected, the game it is in, and its
//!   [`ControlTx`] for answers.
//! - [`ControlTx`]: the S→C side of a session's control stream. Sends
//!   queue and never block; the session's writer puts them on the stream
//!   in order.
//! - [`Sessions`]: every open session's [`ControlTx`] by id, so the lobby
//!   can send to sessions that are not the one it is answering (game list
//!   updates, chat).
//! - [`LobbyApi`]: what the network side asks of the lobby: the lobby's
//!   messages (kinds 0x40-0x5F), the game stream's join tickets, the
//!   games' events, the end of a session, and whether a character is in a
//!   game. The lobby implements it; [`NoLobby`] stands in for a server
//!   without one (every lobby request is `NotSupported`).

use std::collections::BTreeMap;
use std::future::Future;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use d2_net::msg::account::CharSummary;
use d2_net::msg::{ErrorCode, ErrorMsg};
use tokio::sync::mpsc;

use crate::accounts::AccountId;
use crate::denied::Denied;
use crate::games::{GameEvent, GameHandle, Seat};

/// A session's id: unique for the server's run.
pub type SessionId = u64;

/// What one control stream's frames go out by.
#[derive(Clone, Debug)]
pub enum ControlOut {
    /// One frame body.
    Body(Vec<u8>),
    /// End the session: the writer says goodbye and closes the connection.
    Close,
}

/// The S→C side of a session's control stream. Cloning shares it.
#[derive(Clone, Debug)]
pub struct ControlTx {
    id: SessionId,
    tx: mpsc::UnboundedSender<ControlOut>,
}

impl ControlTx {
    /// A control sender and the receiver its session's writer reads.
    pub fn new(id: SessionId) -> (Self, mpsc::UnboundedReceiver<ControlOut>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { id, tx }, rx)
    }

    /// The session it belongs to.
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// Queues one S→C control frame. A session that is gone drops it.
    pub fn send(&self, body: Vec<u8>) {
        let _ = self.tx.send(ControlOut::Body(body));
    }

    /// Answers a failed request of kind `request_kind` with `ERROR`
    /// (kind 0x10). An empty `text` sends the code's own words.
    pub fn error(&self, request_kind: u8, code: ErrorCode, wait_s: u32, text: &str) {
        let mut m = ErrorMsg::new(request_kind, code);
        m.wait_s = wait_s;
        if !text.is_empty() {
            m.text = text.to_owned();
        }
        self.send(m.body());
    }

    /// Ends the session after what is queued.
    pub fn close(&self) {
        let _ = self.tx.send(ControlOut::Close);
    }

    /// Whether the session's writer is gone.
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

/// Every open session's [`ControlTx`]. Cloning shares it.
#[derive(Clone, Debug, Default)]
pub struct Sessions {
    inner: Arc<Mutex<BTreeMap<SessionId, ControlTx>>>,
}

impl Sessions {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<SessionId, ControlTx>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The control sender of session `id`, while it is open.
    pub fn get(&self, id: SessionId) -> Option<ControlTx> {
        self.lock().get(&id).cloned()
    }

    /// Sends `body` to session `id`; false when it is gone.
    pub fn send(&self, id: SessionId, body: Vec<u8>) -> bool {
        match self.get(id) {
            Some(tx) => {
                tx.send(body);
                true
            }
            None => false,
        }
    }

    /// The open sessions' ids, in order.
    pub fn ids(&self) -> Vec<SessionId> {
        self.lock().keys().copied().collect()
    }

    /// How many sessions are open.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    pub(crate) fn insert(&self, tx: ControlTx) {
        self.lock().insert(tx.id(), tx);
    }

    pub(crate) fn remove(&self, id: SessionId) {
        self.lock().remove(&id);
    }
}

/// What the server knows about one session.
#[derive(Debug)]
pub struct SessionCtx {
    pub id: SessionId,
    /// The client's address (its current one; a migrating client keeps
    /// its session).
    pub ip: IpAddr,
    /// The logged-in account and its stored name.
    pub account: Option<(AccountId, String)>,
    /// The selected character's summary (the lobby's rules read it).
    pub character: Option<CharSummary>,
    /// The game the session is in (or holds a slot in).
    pub game: Option<u32>,
    pub out: ControlTx,
}

/// The lobby, as the network side uses it (contracts section 6).
pub trait LobbyApi: Send + Sync + 'static {
    /// A lobby request (kinds 0x40-0x5F), raw. The lobby decodes it and
    /// answers through `s.out`; a `Denied` is sent as `ERROR` for the
    /// request's kind, with its wait and words.
    fn handle(
        &self,
        s: &mut SessionCtx,
        body: &[u8],
    ) -> impl Future<Output = Result<(), Denied>> + Send;

    /// A game stream's `Attach` with a join ticket: once, within 30 s, and
    /// only by the session it was given to; else `TicketInvalid`.
    fn redeem(&self, s: &SessionCtx, ticket: [u8; 16]) -> Result<(GameHandle, Seat), Denied>;

    /// What the games tell the lobby (after the network side has updated
    /// its sessions).
    fn on_game_event(&self, ev: GameEvent);

    /// A session ended (logout, loss past its grace, takeover).
    fn session_ended(&self, s: &SessionCtx);

    /// Whether a character (its lowercase name) is in a game or holds a
    /// slot there.
    fn character_in_game(&self, name_key: &str) -> bool;

    /// What the lobby serves, for the ready line's `caps` (contracts
    /// 10.1): `lobby`, `chat`.
    fn caps(&self) -> &'static [&'static str] {
        &[]
    }
}

/// A server without a lobby: every lobby request is `NotSupported`, and no
/// ticket is ever valid.
#[derive(Debug, Default)]
pub struct NoLobby;

impl LobbyApi for NoLobby {
    async fn handle(&self, _s: &mut SessionCtx, _body: &[u8]) -> Result<(), Denied> {
        Err(ErrorCode::NOT_SUPPORTED.into())
    }

    fn redeem(&self, _s: &SessionCtx, _ticket: [u8; 16]) -> Result<(GameHandle, Seat), Denied> {
        Err(ErrorCode::TICKET_INVALID.into())
    }

    fn on_game_event(&self, _ev: GameEvent) {}

    fn session_ended(&self, _s: &SessionCtx) {}

    fn character_in_game(&self, _name_key: &str) -> bool {
        false
    }
}
