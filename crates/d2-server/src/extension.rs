//! Extension channels: how a mod game adds its own messages without
//! changing the netcode.
//!
//! D2RUST and every mod game share one multiplayer architecture: the same
//! transports, handshake, sessions, join and leave, and the same game
//! messages. A mod that needs messages of its own registers an
//! [`Extension`] on the server's hub for one or more channels. Its
//! messages travel as [`d2_net::Packet::Ext`] on those channels; the
//! bytes are the extension's own format, and nothing else reads them.
//!
//! - **In:** the hub reads a client's extension messages with its other
//!   packets (clients in id order, each client's in arrival order) and
//!   hands them to the channel's extension at the start of the next frame,
//!   before the host drains its queues. So the extension sees them at the
//!   same point of the game whatever the network timing, and a replay of
//!   the same messages gives the same game.
//! - **Out:** an extension answers through an [`ExtOut`]: a message to one
//!   client or to every client, sent with the frame. After a frame that
//!   ran a tick the extension also gets [`Extension::after_tick`], where
//!   it can send what the tick changed.
//! - **Leave:** when a client leaves, every extension hears of it
//!   ([`Extension::client_left`]).
//! - **Unknown channels:** a message on a channel no extension owns is
//!   dropped and listed ([`crate::hub::Hub::take_unrouted`]). D2RUST's own
//!   game registers no extension, so every extension message is unknown
//!   there.

use std::collections::BTreeMap;

use crate::seams::ClientId;

/// One mod extension on the server: its channels and what it does with
/// their messages. `G` is the game the host runs.
pub trait Extension<G> {
    /// The channels this extension owns.
    fn channels(&self) -> Vec<u16>;

    /// One message from `client` on `channel`, at the start of a frame,
    /// in the order the clients sent them.
    fn handle(
        &mut self,
        game: &mut G,
        client: ClientId,
        channel: u16,
        msg: &[u8],
        out: &mut ExtOut,
    );

    /// After a frame that ran a game tick. Default: nothing.
    fn after_tick(&mut self, game: &mut G, out: &mut ExtOut) {
        let _ = (game, out);
    }

    /// `client` has left the game (or its connection was dropped). Default:
    /// nothing.
    fn client_left(&mut self, game: &mut G, client: ClientId) {
        let _ = (game, client);
    }
}

/// Where an extension's messages to the clients go.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExtOut {
    msgs: Vec<(Target, u16, Vec<u8>)>,
}

/// Who an [`ExtOut`] message goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// One client.
    Client(ClientId),
    /// Every connected client (and every client held for a reconnect).
    All,
}

impl ExtOut {
    /// A message to `client` on `channel`.
    pub fn send(&mut self, client: ClientId, channel: u16, msg: Vec<u8>) {
        self.msgs.push((Target::Client(client), channel, msg));
    }

    /// A message to every client on `channel`.
    pub fn broadcast(&mut self, channel: u16, msg: Vec<u8>) {
        self.msgs.push((Target::All, channel, msg));
    }

    /// The messages in the order they were added.
    pub fn take(&mut self) -> Vec<(Target, u16, Vec<u8>)> {
        std::mem::take(&mut self.msgs)
    }
}

/// A channel registration that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExtError {
    /// Another extension owns the channel already.
    #[error("extension channel {0} is taken")]
    ChannelTaken(u16),
}

/// The extensions a hub runs, and which owns each channel.
pub struct Extensions<G> {
    list: Vec<Box<dyn Extension<G> + Send>>,
    owner: BTreeMap<u16, usize>,
}

impl<G> Default for Extensions<G> {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            owner: BTreeMap::new(),
        }
    }
}

impl<G> Extensions<G> {
    /// Adds an extension. Fails, and adds nothing, when one of its
    /// channels is owned already.
    pub fn register(&mut self, ext: Box<dyn Extension<G> + Send>) -> Result<(), ExtError> {
        let channels = ext.channels();
        for (i, &c) in channels.iter().enumerate() {
            if self.owner.contains_key(&c) || channels[..i].contains(&c) {
                return Err(ExtError::ChannelTaken(c));
            }
        }
        let k = self.list.len();
        for c in channels {
            self.owner.insert(c, k);
        }
        self.list.push(ext);
        Ok(())
    }

    /// Whether no extension is registered.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Hands one message to its channel's extension. False when no
    /// extension owns the channel.
    pub fn dispatch(
        &mut self,
        game: &mut G,
        client: ClientId,
        channel: u16,
        msg: &[u8],
        out: &mut ExtOut,
    ) -> bool {
        let Some(&k) = self.owner.get(&channel) else {
            return false;
        };
        self.list[k].handle(game, client, channel, msg, out);
        true
    }

    /// [`Extension::after_tick`] for every extension, in registration
    /// order.
    pub fn after_tick(&mut self, game: &mut G, out: &mut ExtOut) {
        for e in &mut self.list {
            e.after_tick(game, out);
        }
    }

    /// [`Extension::client_left`] for every extension, in registration
    /// order.
    pub fn client_left(&mut self, game: &mut G, client: ClientId) {
        for e in &mut self.list {
            e.client_left(game, client);
        }
    }
}
