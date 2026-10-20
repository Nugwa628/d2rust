//! Connecting to a server: the QUIC connection with the server's pinned
//! certificate, the session handshake on the control stream, and the
//! session's requests and answers.

use std::collections::VecDeque;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use d2_net::msg::{self, ErrorMsg};
use d2_net::wire::{
    parse_attach_answer, parse_session_answer, Attach, ClientHello, ResumeToken, SessionWelcome,
};
use d2_net::Timing;
use d2_quic::{FramedStream, QuicClient, ServerTrust};

use crate::{GameLink, OnlineError};

/// Where a server is and how to know it is the right one: one entry of the
/// launcher's server list (or a typed address in the dev console).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSpec {
    pub id: String,
    pub name: String,
    /// `d2rust`, or a mod game's id.
    pub game: String,
    /// `host:port`.
    pub address: String,
    /// The pinned certificate fingerprint ([`ServerTrust::Any`] only for
    /// tests and the dev console).
    pub trust: ServerTrust,
    /// The game version the server runs.
    pub version: String,
}

/// What this client is, for the handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    /// `d2_proto::PROTOCOL_VERSION`.
    pub protocol: u32,
    /// `d2rust`, or a mod game's id.
    pub game: String,
    /// The four-part game version.
    pub build: String,
    /// The tick rate the client was built for.
    pub tick_rate: u32,
}

/// How [`Session::connect_with`] connects. The game always uses the
/// defaults; the test tool's bots override the hello to check refusals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectOptions {
    /// Sent instead of the hello made from the [`ClientInfo`] (another
    /// build, protocol, tick rate or game).
    pub hello: Option<ClientHello>,
}

/// An open session with a server: the connection and its control stream.
pub struct Session {
    server: ServerSpec,
    me: ClientInfo,
    quic: QuicClient,
    control: FramedStream,
    welcome: SessionWelcome,
    timing: Timing,
    /// Bodies that arrived while a request waited for its answer.
    inbox: VecDeque<Vec<u8>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("server", &self.welcome.server_name)
            .field("peer", &self.quic.peer())
            .finish()
    }
}

fn resolve(address: &str) -> Result<SocketAddr, OnlineError> {
    address
        .to_socket_addrs()
        .map_err(|e| OnlineError::Connect(format!("{address}: {e}")))?
        .next()
        .ok_or_else(|| OnlineError::Connect(format!("{address}: no address")))
}

/// Waits on `stream` for the answer to a handshake or attach.
fn answer(stream: &mut FramedStream, wait: Duration) -> Result<Vec<u8>, OnlineError> {
    stream
        .recv_timeout(wait)
        .map_err(OnlineError::Lost)?
        .ok_or(OnlineError::TimedOut)
}

impl Session {
    /// Connects to `server` and opens a session: the certificate must match
    /// the server's pin, and the server must run what `me` is (protocol,
    /// game, version, tick rate), else it refuses with the reason. Waits
    /// at most about two `timing.timeout`s.
    pub fn connect(
        server: &ServerSpec,
        me: &ClientInfo,
        timing: Timing,
    ) -> Result<Self, OnlineError> {
        Self::open(server, me, None, timing, &ConnectOptions::default())
    }

    /// [`Session::connect`] with `options` (test bots).
    pub fn connect_with(
        server: &ServerSpec,
        me: &ClientInfo,
        timing: Timing,
        options: &ConnectOptions,
    ) -> Result<Self, OnlineError> {
        Self::open(server, me, None, timing, options)
    }

    /// [`Session::connect`] that takes back the session of a lost
    /// connection; its [`SessionWelcome::resume`] says what was kept.
    pub fn resume(
        server: &ServerSpec,
        me: &ClientInfo,
        token: ResumeToken,
        timing: Timing,
    ) -> Result<Self, OnlineError> {
        Self::open(server, me, Some(token), timing, &ConnectOptions::default())
    }

    fn open(
        server: &ServerSpec,
        me: &ClientInfo,
        resume: Option<ResumeToken>,
        timing: Timing,
        options: &ConnectOptions,
    ) -> Result<Self, OnlineError> {
        let addr = resolve(&server.address)?;
        let (quic, mut control) = QuicClient::connect(addr, server.trust.clone(), timing)
            .map_err(|e| OnlineError::Connect(e.to_string()))?;
        let hello = options.hello.clone().unwrap_or_else(|| ClientHello {
            protocol: me.protocol,
            game: me.game.clone(),
            build: me.build.clone(),
            tick_rate: me.tick_rate,
            resume,
        });
        control.send(hello.body()).map_err(OnlineError::Lost)?;
        let body = answer(&mut control, timing.timeout)?;
        let welcome = parse_session_answer(&body)?;
        Ok(Self {
            server: server.clone(),
            me: me.clone(),
            quic,
            control,
            welcome,
            timing,
            inbox: VecDeque::new(),
        })
    }

    /// The server this session is with.
    pub fn server(&self) -> &ServerSpec {
        &self.server
    }

    /// A new connection that takes this session back (after this one was
    /// lost): the same server, client and session token.
    pub fn reconnect(&self) -> Result<Self, OnlineError> {
        Self::open(
            &self.server,
            &self.me,
            Some(self.token()),
            self.timing,
            &ConnectOptions::default(),
        )
    }

    /// The server's welcome: its name, tick rate, whether it takes new
    /// accounts, what a resume kept.
    pub fn welcome(&self) -> &SessionWelcome {
        &self.welcome
    }

    /// The token that takes this session back after a lost connection.
    pub fn token(&self) -> ResumeToken {
        self.welcome.session
    }

    /// Sends one C→S control body.
    pub fn send(&mut self, body: Vec<u8>) -> Result<(), OnlineError> {
        self.control.send(body).map_err(OnlineError::Lost)
    }

    /// The next S→C control body that has arrived (an `ERROR` too), or
    /// `None`.
    pub fn poll(&mut self) -> Result<Option<Vec<u8>>, OnlineError> {
        if let Some(b) = self.inbox.pop_front() {
            return Ok(Some(b));
        }
        self.control.recv().map_err(OnlineError::Lost)
    }

    /// Puts a body back at the front of the queue, for a decoder that
    /// only handles its own kinds.
    pub(crate) fn requeue(&mut self, body: Vec<u8>) {
        self.inbox.push_front(body);
    }

    /// Sends `body` and waits at most `wait` for its answer: the first body
    /// of one of the `answers` kinds. An `ERROR` for this request's kind is
    /// [`OnlineError::Server`]. Anything else that arrives meanwhile (list
    /// updates, chat) stays for [`Session::poll`].
    pub fn request(
        &mut self,
        body: Vec<u8>,
        answers: &[u8],
        wait: Duration,
    ) -> Result<Vec<u8>, OnlineError> {
        let kind = *body.first().ok_or(OnlineError::Malformed)?;
        self.send(body)?;
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Some(b) = self.control.recv_timeout(left).map_err(OnlineError::Lost)? else {
                return Err(OnlineError::TimedOut);
            };
            match b.first() {
                Some(&msg::kind::ERROR) => {
                    let e = ErrorMsg::parse(&b).map_err(|_| OnlineError::Malformed)?;
                    if e.request_kind == kind {
                        return Err(e.into());
                    }
                    self.inbox.push_back(b);
                }
                Some(k) if answers.contains(k) => return Ok(b),
                _ => self.inbox.push_back(b),
            }
        }
    }

    /// Enters the game the lobby's `JoinOk` gave `ticket` for: opens the
    /// game stream and attaches with the ticket.
    pub fn join(&mut self, ticket: [u8; 16]) -> Result<GameLink, OnlineError> {
        self.attach(&Attach::Ticket(ticket))
    }

    /// Comes back to the game slot the server holds after a lost connection
    /// (the game's resume token from its welcome). The server puts the
    /// player back as on a join and sends the whole game again.
    pub fn rejoin(&mut self, token: ResumeToken) -> Result<GameLink, OnlineError> {
        self.attach(&Attach::Resume(token))
    }

    fn attach(&mut self, a: &Attach) -> Result<GameLink, OnlineError> {
        let mut stream = self
            .quic
            .open_stream()
            .map_err(|e| OnlineError::Connect(e.to_string()))?;
        stream.send(a.body()).map_err(OnlineError::Lost)?;
        let body = answer(&mut stream, self.timing.timeout)?;
        let welcome = parse_attach_answer(&body)?;
        Ok(GameLink::attached(stream, welcome))
    }

    /// Ends the session (a logout): a goodbye on the control stream, then
    /// the connection closes.
    pub fn close(mut self) {
        self.control.close();
        self.quic.close();
    }
}
