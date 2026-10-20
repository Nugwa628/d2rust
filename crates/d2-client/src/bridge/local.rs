//! The in-process server link of single player: [`ServerLink`] over a
//! `d2_server::hub::Hub` and `d2-net`'s in-memory transport. Single player
//! runs the same authoritative server as online play, in this process: the
//! bridge's sends travel as `d2_net::Packet`s to the hub, which hands them
//! to the host, and what the host delivers comes back the same way.
//!
//! The in-memory transport carries a packet at once, so the link has the
//! hub read each send as soon as it is made and deliver as soon as the
//! bridge receives: every host call happens at the same moment, and in the
//! same order, as a direct call would. The host owns the clock, the game
//! sender's duplicate filter, the queues, the tick driver and the flush;
//! this adapter only forwards and never interprets a message.

use d2_net::{pair, Lane, MemoryEnd, Packet, Transport};
use d2_proto::PROTOCOL_VERSION;
use d2_server::adapters::ProtoSizes;
use d2_server::host::{FrameReport as HostFrame, Host, HostError, SystemClock};
use d2_server::hub::{Accepted, Hub, HubError};
use d2_server::seams::{Clock, Intents, MessageSink, MessageSizes, SessionHandler, Tick};
use d2_server::transport::{Classified, SendError};

use super::link::{LinkError, Pumped, SendQueue, Sent, ServerLink, LOCAL_CLIENT};

/// A failure of the local host, as the link reports it.
#[derive(Debug, thiserror::Error)]
pub enum LocalError {
    /// A send the recorded client transport asserts on.
    #[error(transparent)]
    Send(#[from] SendError),
    /// A host frame the recorded client ends with a fatal assert.
    #[error(transparent)]
    Host(#[from] HostError),
    /// The in-memory connection to the hub failed.
    #[error(transparent)]
    Hub(HubError),
    /// The server classifier did not queue a message the bridge's
    /// classifier passed (both read `d2-proto`'s size rules, so this is
    /// a table disagreement, never a client decision).
    #[error("server classifier dropped message 0x{id:02X}: {class:?}")]
    Dropped { id: u8, class: Classified },
}

impl From<HubError> for LocalError {
    fn from(e: HubError) -> Self {
        match e {
            HubError::Host(e) => LocalError::Host(e),
            e => LocalError::Hub(e),
        }
    }
}

impl From<LocalError> for LinkError {
    fn from(e: LocalError) -> Self {
        LinkError::Server(Box::new(e))
    }
}

/// The system-message handler until the session code exists: records each
/// drained system message (ids 0x67..=0x70) and answers only the ping. It
/// decides no outcome.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingSession {
    /// (client, drained bytes, full size), in drain order.
    pub received: Vec<(u32, Vec<u8>, usize)>,
}

/// S→C 0x8F: the id and 32 zero bytes (0x21 bytes;
/// `sim/intents-events.md` §2.5 row 0x6D).
const PONG: [u8; 33] = {
    let mut b = [0; 33];
    b[0] = 0x8F;
    b
};

impl SessionHandler for PendingSession {
    fn system_message(&mut self, client: u32, msg: &[u8], size: usize, out: &mut dyn MessageSink) {
        self.received.push((client, msg.to_vec(), size));
        // C→S 0x6D (ping): the server logs the
        // round trip and answers the pong. The bytes of the ping are data
        // (the client's clock); the answer does not depend on them.
        if msg.first() == Some(&0x6D) {
            let _ = out.queue(client, &PONG);
        }
    }
}

/// The in-process `d2-server` host behind the bridge, for the local
/// player ([`LOCAL_CLIENT`]), connected through the in-memory transport.
pub struct LocalLink<G, S = ProtoSizes, H = PendingSession, C = SystemClock> {
    hub: Hub<G, S, H, C, MemoryEnd>,
    /// The client's end of the in-memory connection.
    end: MemoryEnd,
    /// The server part of the last pump (diagnostics and tests).
    last: HostFrame,
    /// Run in each pump after the tick, before its flush
    /// ([`Host::frame_with`]; the state-dump pokes use it).
    tick_end: Option<TickEndHook<G, S, H, C>>,
    /// Extension-channel messages from the server not yet taken, in
    /// arrival order ([`LocalLink::take_ext`]).
    ext_in: Vec<(u16, Vec<u8>)>,
}

/// A [`LocalLink`] tick-end hook.
pub type TickEndHook<G, S, H, C> = Box<dyn FnMut(&mut Host<G, S, H, C>) + Send>;

/// The single-player link of the app: `d2-proto` sizes, the real clock.
pub type SinglePlayer<G> = LocalLink<G, ProtoSizes, PendingSession, SystemClock>;

impl<G> SinglePlayer<G>
where
    G: Intents + Tick,
{
    /// A local server running `game` on the system clock.
    pub fn single_player(game: G) -> Self {
        Self::new(Host::new(
            game,
            ProtoSizes,
            PendingSession::default(),
            SystemClock::default(),
        ))
    }
}

impl<G, S, H, C> LocalLink<G, S, H, C>
where
    G: Intents + Tick,
    S: MessageSizes,
    H: SessionHandler,
    C: Clock,
{
    /// Wraps `host` in a hub and connects the local client to it over the
    /// in-memory transport. The game must already list the client
    /// (`SimGame::join`) for its messages to pass the gate and for the
    /// flush to reach it.
    pub fn new(host: Host<G, S, H, C>) -> Self {
        let mut hub = Hub::new(host);
        let (end, server_end) = pair();
        hub.connect(LOCAL_CLIENT, server_end);
        // Spec: specs/sim/intents-events.md §3.3 rule 5: server start sends
        // S→C 0xAF once to client 0; it sets the connection flag (§3.4 rule 2).
        // The packet recorder takes it, rather than the receive lists.
        hub.host.announce_direct(LOCAL_CLIENT, &[0xAF, 0x00]);
        Self {
            hub,
            end,
            last: HostFrame::default(),
            tick_end: None,
            ext_in: Vec::new(),
        }
    }

    /// Installs `f` (replacing an earlier one), run in every pump that
    /// ticks, after the tick and before its flush.
    pub fn set_tick_end(&mut self, f: TickEndHook<G, S, H, C>) {
        self.tick_end = Some(f);
    }

    pub fn host(&self) -> &Host<G, S, H, C> {
        &self.hub.host
    }

    pub fn host_mut(&mut self) -> &mut Host<G, S, H, C> {
        &mut self.hub.host
    }

    /// The server's mod extensions, where a mod game registers its own
    /// (`d2_server::extension`). D2RUST's own game has none.
    pub fn extensions_mut(&mut self) -> &mut d2_server::extension::Extensions<G> {
        &mut self.hub.extensions
    }

    /// Sends a mod extension's message on `channel` (`d2_server::extension`).
    /// The server's extension gets it at the start of the next frame.
    pub fn send_ext(&mut self, channel: u16, msg: Vec<u8>) -> Result<(), LinkError> {
        self.end
            .send(Packet::Ext { channel, msg })
            .map_err(|source| HubError::Net {
                client: LOCAL_CLIENT,
                source,
            })
            .map_err(LocalError::from)?;
        self.hub
            .poll_client(LOCAL_CLIENT)
            .map_err(LocalError::from)?;
        Ok(())
    }

    /// The extension messages the server sent since the last call:
    /// channel and bytes, in arrival order. A mod's client side reads
    /// them after each pump.
    pub fn take_ext(&mut self) -> Vec<(u16, Vec<u8>)> {
        std::mem::take(&mut self.ext_in)
    }

    /// The server part of the last pump: drained messages and their
    /// fate, tick, flush.
    pub fn last_frame(&self) -> &HostFrame {
        &self.last
    }

    fn queued(msg: &[u8], class: Classified) -> Result<Sent, LinkError> {
        match class {
            Classified::Queued(_) => Ok(Sent::Queued),
            class => Err(LocalError::Dropped { id: msg[0], class }.into()),
        }
    }

    /// Reads the client's end until nothing is waiting: the delivered
    /// messages, in order. Frame marks are skipped: the pump reports its
    /// tick itself. Extension messages are kept for [`Self::take_ext`].
    fn take_delivered(&mut self) -> Result<Vec<Vec<u8>>, LocalError> {
        let mut out = Vec::new();
        while let Some(p) = self
            .end
            .recv()
            .map_err(|source| HubError::Net {
                client: LOCAL_CLIENT,
                source,
            })
            .map_err(LocalError::from)?
        {
            match p {
                Packet::Deliver(msg) => out.push(msg),
                Packet::Ext { channel, msg } => self.ext_in.push((channel, msg)),
                _ => {}
            }
        }
        Ok(out)
    }
}

impl<G, S, H, C> ServerLink for LocalLink<G, S, H, C>
where
    G: Intents + Tick,
    S: MessageSizes,
    H: SessionHandler,
    C: Clock,
{
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.hub.host.driver.rate()
    }

    fn server_frame(&mut self) -> Option<i32> {
        Some(self.hub.host.game.frame())
    }

    /// Sends the message over the transport and has the hub read it at
    /// once: the game sender (with its duplicate filter) or the system
    /// sender.
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        let lane = match queue {
            SendQueue::Game => Lane::Game,
            SendQueue::System => Lane::System,
        };
        self.end
            .send(Packet::Send {
                lane,
                msg: msg.to_vec(),
            })
            .map_err(|source| HubError::Net {
                client: LOCAL_CLIENT,
                source,
            })
            .map_err(LocalError::from)?;
        let polled = self
            .hub
            .poll_client(LOCAL_CLIENT)
            .map_err(LocalError::from)?;
        // One packet in, one result out.
        let Some(p) = polled.into_iter().next() else {
            return Ok(Sent::Queued);
        };
        match p.result.map_err(LocalError::from)? {
            Accepted::Filtered => Ok(Sent::Filtered),
            Accepted::Classified(class) => Self::queued(msg, class),
        }
    }

    /// One hub frame: drain → tick driver → flush if a tick ran.
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        let hook = &mut self.tick_end;
        self.last = self
            .hub
            .frame_with(|h| {
                if let Some(f) = hook.as_mut() {
                    f(h)
                }
            })
            .map_err(LocalError::from)?;
        // Nothing is delivered between receives, so the end holds only
        // this frame's mark; the report above already says if it ticked.
        self.take_delivered()?;
        Ok(Pumped {
            ticked: self.last.ticked,
        })
    }

    fn set_display_catch_up(&mut self, enabled: bool) {
        self.hub.host.display_lag_ms = enabled.then_some(250);
    }

    fn tick_pending(&mut self) -> bool {
        self.hub
            .host
            .driver
            .next_tick_ms()
            .is_some_and(|next| next <= self.hub.host.base_ms())
    }

    /// What the host delivered to the local client since the last call:
    /// the system list first, one message per chunk.
    fn receive(&mut self) -> Vec<Vec<u8>> {
        // The in-memory connection cannot fail while the link holds both
        // ends, so a failure here is a broken invariant.
        self.hub
            .deliver(LOCAL_CLIENT)
            .expect("in-memory delivery to the local client");
        self.take_delivered()
            .expect("in-memory delivery to the local client")
    }

    /// `Host::set_send_now`.
    fn set_send_now(&mut self, now: Option<u32>) {
        self.hub.host.set_send_now(now)
    }
}
