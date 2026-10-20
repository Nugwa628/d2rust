//! The server link: the narrow interface the bridge needs from the
//! in-process `d2-server` host (or, later, a remote one over `d2-net`).

use d2_proto::transport::ClientQueue;

/// Client id of the local player (Inputs).
pub const LOCAL_CLIENT: u32 = 0;

/// Server queue a C→S message is handed to. Queue 2 (admin)
/// is never sent by the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendQueue {
    /// Ids below 0x67: the client's game-message sender.
    Game,
    /// Ids 0x67..=0x70: the system-message senders.
    System,
}

impl SendQueue {
    /// The send queue of a classifier queue; `None` for the admin queue.
    pub fn of(queue: ClientQueue) -> Option<Self> {
        match queue {
            ClientQueue::Game => Some(Self::Game),
            ClientQueue::System => Some(Self::System),
            ClientQueue::Admin => None,
        }
    }
}

/// What the link did with a sent message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    /// Appended to the server queue.
    Queued,
    /// Dropped by the client sender's duplicate filter.
    Filtered,
}

/// One server frame (`pump`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pumped {
    /// The tick driver ran a tick (and the flush ran).
    pub ticked: bool,
}

/// A failure inside the link or the server behind it.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("server: {0}")]
    Server(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// The bridge's view of the server. Implemented by the
/// in-process host adapter once `d2-server` is wired, and by test fakes.
pub trait ServerLink {
    /// The server's `d2_proto::PROTOCOL_VERSION`.
    fn protocol_version(&self) -> u32;

    /// The game's tick rate: the server runs one tick every `1 / hz`
    /// seconds, and the client steps its own model (walk prediction,
    /// paths, animations, client missiles, timers) at the same rate.
    /// Fixed for the link's game; no wire message carries it, the link
    /// knows its host. No default, so no link runs the client at the base
    /// rate by accident: test fakes say `TickRate::BASE` on purpose.
    fn tick_rate(&self) -> d2_sim::tick::TickRate;

    /// Authoritative game frame; remote links without that information return none.
    fn server_frame(&mut self) -> Option<i32> {
        None
    }

    /// Hands one C→S message to the server's sender for `queue`.
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError>;

    /// One server frame: drain → tick driver → flush if a tick ran.
    fn pump(&mut self) -> Result<Pumped, LinkError>;

    /// Selects bounded catch-up for a display frame. Single-tick harnesses
    /// leave this off. Remote links manage their own clock.
    fn set_display_catch_up(&mut self, _enabled: bool) {}

    /// Whether another local tick is due at the last sampled host time.
    fn tick_pending(&mut self) -> bool {
        false
    }

    /// The S→C chunks delivered to the local client since the last call,
    /// system list first, each as delivered.
    fn receive(&mut self) -> Vec<Vec<u8>>;

    /// The server monsters' path state after the last tick, for the
    /// single-player server copy:
    /// only a link that holds the game in process has them (the client
    /// reads the server's units directly there); the default has none.
    fn server_paths(&mut self) -> Vec<super::critter_path::ServerPath> {
        Vec::new()
    }

    /// The same read for the client update's own path step,
    /// taken before the update: a link
    /// whose host copies after the update itself (the dump) answers here
    /// only. The default is [`ServerLink::server_paths`].
    fn step_paths(&mut self) -> Vec<super::critter_path::ServerPath> {
        self.server_paths()
    }

    /// The client sender's clock for its duplicate filter
    /// (`d2_server::host::Host::set_send_now`): a check run replays the
    /// recorded host clock; `None` goes back to the host's. Only a link
    /// that holds the game in process has a sender; the default does
    /// nothing.
    fn set_send_now(&mut self, _now: Option<u32>) {}

    /// The server runs elsewhere: its answers arrive on the network's time,
    /// not within the frame that asked (the leave waits for them in real
    /// time).
    fn is_remote(&self) -> bool {
        false
    }

    /// The server re-entered this client's game after a lost connection
    /// since the last call (contracts 3.7): the game state the link hands
    /// over next rebuilds everything, so the bridge clears its world before
    /// it applies any of it. The default: never.
    fn take_resume(&mut self) -> bool {
        false
    }
}
