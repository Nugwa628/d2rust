//! The server's side of its client connections: a [`Hub`] holds the
//! [`Host`] and one [`Transport`] end per connected client, and moves
//! messages between the two.
//!
//! - **In:** [`Hub::poll`] reads every client's arrived packets and hands
//!   each C→S message to the host's sender for its lane (the game sender
//!   with its duplicate filter, or the system sender). Clients are read in
//!   `ClientId` order and each client's packets in arrival order, so the
//!   server queues get the same order whatever the network timing.
//! - **Out:** [`Hub::deliver`] moves what the host delivered to a client
//!   (its receive lists: system messages first, then game messages) onto
//!   the client's transport, and [`Hub::frame`] runs one host frame and
//!   tells every client whether a tick ran.
//! - **Lost connections:** when a client's connection closes, times out or
//!   breaks the wire format while the hub serves all clients
//!   ([`Hub::poll`], [`Hub::deliver_all`], [`Hub::frame`]), the other
//!   clients play on. Without a grace period the client is disconnected at
//!   once and listed in [`Hub::take_dropped`].
//! - **Reconnect grace:** with a grace period ([`Hub::set_grace`]) a client
//!   connected with a resume token ([`Hub::connect_with_token`]) is parked
//!   instead: it stays in the game (its player stands where it was), and
//!   what the server sends it meanwhile is kept. If it comes back with its
//!   token within the grace period ([`Hub::resume`]) it gets its slot and
//!   the kept messages; otherwise it is disconnected and listed in
//!   [`Hub::take_dropped`] when the period ends.
//!
//! - **Extension channels:** a client's [`Packet::Ext`] messages go to
//!   the mod extension that owns their channel ([`crate::extension`]),
//!   at the start of the next frame, in the same client and arrival order;
//!   what the extensions send goes out with the frame.
//!
//! Single player uses the same path over the in-memory transport: the
//! client's link polls and delivers right after each call, so every host
//! call happens at the same moment and in the same order as a direct call
//! would.

use std::collections::BTreeMap;

use d2_net::{Lane, NetError, Packet, ResumeToken, Transport};

use crate::extension::{ExtOut, Extensions, Target};
use crate::host::{FrameReport, Host, HostError};
use crate::seams::{ClientId, Clock, Intents, MessageSizes, SessionHandler, Tick};
use crate::transport::{Classified, SendError};

/// What the host's sender did with one C→S message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accepted {
    /// The classifier gave this result (queued, or dropped and why).
    Classified(Classified),
    /// The game sender's duplicate filter dropped it.
    Filtered,
}

/// One C→S message the hub handed to the host, in order.
#[derive(Debug, PartialEq, Eq)]
pub struct Polled {
    /// The sending client.
    pub client: ClientId,
    /// The sender it went through.
    pub lane: Lane,
    /// The first byte of the message (0 for an empty one).
    pub id: u8,
    /// What the sender did with it, or the size limit it broke.
    pub result: Result<Accepted, SendError>,
}

/// A hub failure: a client's connection, or the host.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum HubError {
    /// No client with this id is connected.
    #[error("client {0} is not connected")]
    UnknownClient(ClientId),
    /// The client sent a packet only the server sends.
    #[error("client {client} sent a server-only packet")]
    Unexpected {
        /// The client.
        client: ClientId,
    },
    /// The client's connection failed.
    #[error("client {client}: {source}")]
    Net {
        /// The client.
        client: ClientId,
        /// What failed.
        source: NetError,
    },
    /// The host failed (a state a consistent host never reaches).
    #[error(transparent)]
    Host(#[from] HostError),
}

/// The host and its client connections.
pub struct Hub<G, S, H, C, T = Box<dyn Transport + Send>> {
    /// The server host the messages go to and come from.
    pub host: Host<G, S, H, C>,
    clients: BTreeMap<ClientId, T>,
    /// Resume tokens of the clients that may come back after a loss.
    tokens: BTreeMap<ClientId, ResumeToken>,
    /// Clients whose connection was lost, held for the grace period.
    parked: BTreeMap<ClientId, Parked>,
    /// How long a parked client is held, in host clock milliseconds
    /// (0: lost clients are disconnected at once).
    grace_ms: u32,
    /// Clients disconnected because their connection failed, in order.
    dropped: Vec<(ClientId, NetError)>,
    /// The mod extensions and the channels they own (none in D2RUST's
    /// own game).
    pub extensions: Extensions<G>,
    /// Extension messages read since the last frame, in read order.
    ext_in: Vec<(ClientId, u16, Vec<u8>)>,
    /// Extension messages on channels no extension owns, in order.
    unrouted: Vec<(ClientId, u16)>,
}

/// Most packets kept for a parked client. One that misses more is
/// disconnected: it couldn't catch up anyway.
pub const MAX_BACKLOG: usize = 1 << 16;

/// A client whose connection was lost, waiting to come back.
#[derive(Debug)]
struct Parked {
    /// Host clock when the loss was noticed.
    since_ms: u32,
    /// What failed.
    cause: NetError,
    /// What the server sent it since, in order.
    backlog: Vec<Packet>,
}

impl<G, S, H, C, T> Hub<G, S, H, C, T>
where
    G: Intents + Tick,
    S: MessageSizes,
    H: SessionHandler,
    C: Clock,
    T: Transport,
{
    /// A hub around `host` with no clients.
    pub fn new(host: Host<G, S, H, C>) -> Self {
        Self {
            host,
            clients: BTreeMap::new(),
            tokens: BTreeMap::new(),
            parked: BTreeMap::new(),
            grace_ms: 0,
            dropped: Vec::new(),
            extensions: Extensions::default(),
            ext_in: Vec::new(),
            unrouted: Vec::new(),
        }
    }

    /// Sets the reconnect grace period, in host clock milliseconds (0
    /// turns it off).
    pub fn set_grace(&mut self, ms: u32) {
        self.grace_ms = ms;
    }

    /// [`Hub::connect`] for a client that may come back after a lost
    /// connection by presenting `token` ([`Hub::resume`]).
    pub fn connect_with_token(&mut self, client: ClientId, transport: T, token: ResumeToken) {
        self.connect(client, transport);
        self.tokens.insert(client, token);
    }

    /// The client a resume token belongs to, if the hub still holds that
    /// client (parked, or connected and not yet noticed as lost).
    pub fn resumable(&self, token: &ResumeToken) -> Option<ClientId> {
        self.tokens
            .iter()
            .find(|(_, t)| *t == token)
            .map(|(&c, _)| c)
    }

    /// Gives a returning client its slot back over a new `transport`:
    /// what it missed while parked goes out first. Returns the client, or
    /// `None` when no held client has this token. A client whose old
    /// connection is still open has it replaced (and closed).
    pub fn resume(&mut self, token: &ResumeToken, mut transport: T) -> Option<ClientId> {
        let client = self.resumable(token)?;
        if let Some(p) = self.parked.get(&client) {
            for packet in &p.backlog {
                if transport.send(packet.clone()).is_err() {
                    // The new connection failed already: still parked.
                    return None;
                }
            }
            self.parked.remove(&client);
        }
        self.clients.insert(client, transport);
        Some(client)
    }

    /// Gives a returning client its slot back over a new `transport`
    /// without replaying what it missed: the game sends its state again
    /// (a re-entry, contracts 3.7). The client gets `new_token` for a
    /// later loss; the old one stops working. Returns the client, or
    /// `None` when no held client has this token.
    pub fn reattach(
        &mut self,
        token: &ResumeToken,
        new_token: ResumeToken,
        transport: T,
    ) -> Option<ClientId> {
        let client = self.resumable(token)?;
        self.parked.remove(&client);
        self.tokens.insert(client, new_token);
        self.clients.insert(client, transport);
        Some(client)
    }

    /// The clients parked in their grace period, in id order.
    pub fn parked(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.parked.keys().copied()
    }

    /// Connects `client` over `transport`: the host registers it
    /// ([`Host::connect`]) and its packets are read from now on. A client
    /// already connected gets the new transport.
    pub fn connect(&mut self, client: ClientId, transport: T) {
        self.host.connect(client);
        self.clients.insert(client, transport);
    }

    /// Disconnects `client`: the host forgets it, the extensions hear of
    /// it, and its transport is returned (dropping it closes the
    /// connection). Its unhandled extension messages go with it.
    pub fn disconnect(&mut self, client: ClientId) -> Option<T> {
        self.host.disconnect(client);
        self.ext_in.retain(|(c, _, _)| *c != client);
        self.extensions.client_left(&mut self.host.game, client);
        self.tokens.remove(&client);
        self.parked.remove(&client);
        self.clients.remove(&client)
    }

    /// The connected clients (not the parked ones), in id order.
    pub fn clients(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients.keys().copied()
    }

    /// The clients disconnected since the last call because their
    /// connection failed (and their grace period, if any, ran out), with
    /// what failed, in order.
    pub fn take_dropped(&mut self) -> Vec<(ClientId, NetError)> {
        std::mem::take(&mut self.dropped)
    }

    /// The extension messages dropped since the last call because no
    /// extension owns their channel: sender and channel, in order.
    pub fn take_unrouted(&mut self) -> Vec<(ClientId, u16)> {
        std::mem::take(&mut self.unrouted)
    }

    /// Disconnects a client whose connection failed and lists it.
    fn drop_client(&mut self, client: ClientId, e: NetError) {
        self.disconnect(client);
        self.dropped.push((client, e));
    }

    /// A client's connection failed: it is parked when it can come back
    /// (a grace period is set, it has a token, and the connection was lost
    /// rather than broken), and dropped otherwise.
    fn lose_client(&mut self, client: ClientId, e: NetError) {
        let lost = matches!(e, NetError::Closed | NetError::TimedOut);
        if self.grace_ms == 0 || !lost || !self.tokens.contains_key(&client) {
            return self.drop_client(client, e);
        }
        self.clients.remove(&client);
        let since_ms = self.host.clock.now_ms();
        self.parked.insert(
            client,
            Parked {
                since_ms,
                cause: e,
                backlog: Vec::new(),
            },
        );
    }

    /// Keeps a packet for a parked client; one that missed too much is
    /// dropped.
    fn keep_for(&mut self, client: ClientId, packet: Packet) {
        let Some(p) = self.parked.get_mut(&client) else {
            return;
        };
        if p.backlog.len() >= MAX_BACKLOG {
            let cause = p.cause.clone();
            return self.drop_client(client, cause);
        }
        p.backlog.push(packet);
    }

    /// Disconnects the parked clients whose grace period has run out.
    fn expire_parked(&mut self) {
        if self.parked.is_empty() {
            return;
        }
        let now = self.host.clock.now_ms();
        let over: Vec<(ClientId, NetError)> = self
            .parked
            .iter()
            .filter(|(_, p)| now.wrapping_sub(p.since_ms) >= self.grace_ms)
            .map(|(&c, p)| (c, p.cause.clone()))
            .collect();
        for (client, cause) in over {
            self.drop_client(client, cause);
        }
    }

    /// Reads every client's arrived packets, clients in id order and each
    /// client's in arrival order, and hands each C→S message to the host.
    /// Returns what the host's senders did with each, in that order. A
    /// client whose connection failed, or that sent a server-only packet,
    /// is lost (what it sent before the failure still counts): parked when
    /// it can come back, dropped otherwise.
    pub fn poll(&mut self) -> Result<Vec<Polled>, HubError> {
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        let mut out = Vec::new();
        for client in ids {
            match self.poll_client_into(client, &mut out) {
                Ok(()) => {}
                Err(HubError::Net { source, .. }) => self.lose_client(client, source),
                Err(HubError::Unexpected { .. }) => self.drop_client(client, NetError::Malformed),
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// [`Hub::poll`] for one client only.
    pub fn poll_client(&mut self, client: ClientId) -> Result<Vec<Polled>, HubError> {
        let mut out = Vec::new();
        self.poll_client_into(client, &mut out)?;
        Ok(out)
    }

    fn poll_client_into(
        &mut self,
        client: ClientId,
        out: &mut Vec<Polled>,
    ) -> Result<(), HubError> {
        loop {
            let transport = self
                .clients
                .get_mut(&client)
                .ok_or(HubError::UnknownClient(client))?;
            let packet = transport
                .recv()
                .map_err(|source| HubError::Net { client, source })?;
            let Some(packet) = packet else {
                return Ok(());
            };
            let (lane, msg) = match packet {
                Packet::Send { lane, msg } => (lane, msg),
                Packet::Ext { channel, msg } => {
                    self.ext_in.push((client, channel, msg));
                    continue;
                }
                _ => return Err(HubError::Unexpected { client }),
            };
            let result = match lane {
                Lane::Game => self
                    .host
                    .send_game(client, &msg)
                    .map(|c| c.map_or(Accepted::Filtered, Accepted::Classified)),
                Lane::System => self
                    .host
                    .send_system(client, &msg)
                    .map(Accepted::Classified),
            };
            out.push(Polled {
                client,
                lane,
                id: msg.first().copied().unwrap_or(0),
                result,
            });
        }
    }

    /// Moves everything the host delivered to `client` onto its transport,
    /// in receive order (system messages first, then game messages).
    /// Returns how many messages went.
    pub fn deliver(&mut self, client: ClientId) -> Result<usize, HubError> {
        let transport = self
            .clients
            .get_mut(&client)
            .ok_or(HubError::UnknownClient(client))?;
        let msgs = self.host.receive(client);
        let n = msgs.len();
        for msg in msgs {
            transport
                .send(Packet::Deliver(msg))
                .map_err(|source| HubError::Net { client, source })?;
        }
        Ok(n)
    }

    /// [`Hub::deliver`] for every client, in id order, then the end of the
    /// frame ([`Transport::flush`]). A client whose connection failed is
    /// lost; a parked client keeps what it was sent.
    pub fn deliver_all(&mut self) {
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for client in ids {
            let msgs = self.host.receive(client);
            let mut rest = msgs.into_iter().map(Packet::Deliver);
            let failed = {
                let transport = self.clients.get_mut(&client).expect("a connected client");
                rest.by_ref()
                    .find_map(|p| transport.send(p.clone()).err().map(|e| (p, e)))
            };
            if let Some((p, e)) = failed {
                self.lose_client(client, e);
                for p in std::iter::once(p).chain(rest) {
                    self.keep_for(client, p);
                }
                continue;
            }
            let flushed = self
                .clients
                .get_mut(&client)
                .expect("a connected client")
                .flush();
            if let Err(e) = flushed {
                self.lose_client(client, e);
            }
        }
        let parked: Vec<ClientId> = self.parked.keys().copied().collect();
        for client in parked {
            for msg in self.host.receive(client) {
                self.keep_for(client, Packet::Deliver(msg));
            }
        }
    }

    /// One host frame ([`Host::frame_with`]): drain, tick when due, flush
    /// after a tick. Every client is then sent a [`Packet::Frame`] mark
    /// (kept for parked ones); a client whose connection failed is lost,
    /// and parked clients whose grace period ran out are dropped.
    pub fn frame_with(
        &mut self,
        at_tick_end: impl FnOnce(&mut Host<G, S, H, C>),
    ) -> Result<FrameReport, HubError> {
        let mut out = ExtOut::default();
        for (client, channel, msg) in std::mem::take(&mut self.ext_in) {
            if !self
                .extensions
                .dispatch(&mut self.host.game, client, channel, &msg, &mut out)
            {
                self.unrouted.push((client, channel));
            }
        }
        let report = self.host.frame_with(at_tick_end)?;
        if report.ticked {
            self.extensions.after_tick(&mut self.host.game, &mut out);
        }
        self.send_ext(out);
        let mark = Packet::Frame {
            ticked: report.ticked,
        };
        let mut failed = Vec::new();
        for (&client, transport) in &mut self.clients {
            if let Err(e) = transport.send(mark.clone()) {
                failed.push((client, e));
            }
        }
        for (client, e) in failed {
            self.lose_client(client, e);
        }
        let parked: Vec<ClientId> = self.parked.keys().copied().collect();
        for client in parked {
            self.keep_for(client, mark.clone());
        }
        self.expire_parked();
        Ok(report)
    }

    /// Sends the extensions' messages, in order, before the frame mark: to
    /// one client or to all (parked clients keep theirs). A client that is
    /// gone gets nothing; one whose connection fails is lost.
    fn send_ext(&mut self, mut out: ExtOut) {
        for (target, channel, msg) in out.take() {
            let to: Vec<ClientId> = match target {
                Target::Client(c) => vec![c],
                Target::All => self
                    .clients
                    .keys()
                    .chain(self.parked.keys())
                    .copied()
                    .collect(),
            };
            for client in to {
                let p = Packet::Ext {
                    channel,
                    msg: msg.clone(),
                };
                if self.parked.contains_key(&client) {
                    self.keep_for(client, p);
                } else if let Some(t) = self.clients.get_mut(&client) {
                    if let Err(e) = t.send(p.clone()) {
                        self.lose_client(client, e);
                        self.keep_for(client, p);
                    }
                }
            }
        }
    }

    /// [`Hub::frame_with`] with nothing run at the tick end.
    pub fn frame(&mut self) -> Result<FrameReport, HubError> {
        self.frame_with(|_| {})
    }

    /// One frame of a server that owns its frame loop (see
    /// [`crate::game_thread::GameThread::spawn_looped`]): read every
    /// client ([`Hub::poll`]), run the host frame ([`Hub::frame`]), then
    /// deliver what the host has for every client ([`Hub::deliver_all`]).
    /// This is the order the single-player link uses: sends are read
    /// before the frame, and deliveries follow the frame mark.
    pub fn serve_frame(&mut self) -> Result<FrameReport, HubError> {
        self.poll()?;
        let report = self.frame()?;
        self.deliver_all();
        Ok(report)
    }
}
