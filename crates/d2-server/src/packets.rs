//! The packet recorder of D2RUST: an optional observer on [`crate::host::Host`]
//! that sees every client → server message where the server takes it,
//! every dispatch and result, the tick markers, every server → client
//! message as queued, every direct send and every flushed buffer, in the
//! order they happen. [`PacketLog`] turns them into `packets-raw-1` lines,
//! the packet-trace format the scenario tools compare, with the frame and
//! the loop phase on each record.
//!
//! Recording never changes the game: the host only reads (copies of the
//! messages, `Intents::frame`, `Intents::player`), and with no observer
//! nothing is copied at all.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};

use crate::seams::{ClientId, ResultCode};

/// One recorded event (the record `type` in brackets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketEvent<'a> {
    /// The client's game-message sender, before its duplicate filter
    /// (`client_send`).
    ClientSend { client: ClientId, msg: &'a [u8] },
    /// What leaves the client for the server queues (`client_out`).
    ClientOut { client: ClientId, msg: &'a [u8] },
    /// The drain of the server queues starts (`drain`).
    Drain,
    /// A drained message (`c2s` for the game queue, `c2s_sys` for the
    /// system queue): the drain copy and the full size.
    C2s {
        system: bool,
        client: ClientId,
        size: usize,
        msg: &'a [u8],
    },
    /// The dispatcher is called (`dispatch`): id, full size, the game
    /// frame it reads.
    Dispatch {
        client: ClientId,
        id: u8,
        size: usize,
        game_frame: i32,
    },
    /// The dispatcher's result code (`result`).
    Result { client: ClientId, code: ResultCode },
    /// A tick starts; `frame` = the frame after its increment (`tick`).
    Tick { frame: i32 },
    /// The tick returned (`tick_end`).
    TickEnd { frame: i32 },
    /// The flush is called (`flush`).
    Flush,
    /// A message queued for a client (`s2c`).
    S2c { client: ClientId, msg: &'a [u8] },
    /// A buffer handed to local delivery by a flush, or a direct send
    /// (`net`).
    Net {
        client: ClientId,
        msg: &'a [u8],
        direct: bool,
    },
}

/// Receives the host's packet events (installed with
/// [`crate::host::Host::set_packet_observer`]).
pub trait PacketObserver {
    /// Called once per event, in the order the events happen.
    fn packet(&mut self, ev: PacketEvent<'_>);
}

/// The `caller` field of a flushed buffer's `net` record: a fixed label
/// for the flush's net send, so packet files from both sides read the
/// same way.
pub const FLUSH_CALLER: &str = "0x52e3b5";

#[derive(Debug, Default)]
struct Shared {
    lines: Vec<String>,
    counts: BTreeMap<&'static str, u64>,
    seq: u64,
}

/// The reader's handle on a [`PacketLog`]'s lines (the log itself lives
/// on the server thread, inside the host).
#[derive(Clone, Debug, Default)]
pub struct PacketLines(Arc<Mutex<Shared>>);

impl PacketLines {
    /// The lines written since the last take, in order.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner).lines)
    }

    /// Records per `type` so far.
    pub fn counts(&self) -> BTreeMap<String, u64> {
        let s = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        s.counts
            .iter()
            .map(|(k, v)| ((*k).to_owned(), *v))
            .collect()
    }

    /// Records written so far (the next `seq`).
    pub fn events(&self) -> u64 {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).seq
    }
}

/// Writes `packets-raw-1` record lines: the `frame` of the last tick (null
/// before the first) and the loop phase after the record's event.
#[derive(Debug)]
pub struct PacketLog {
    shared: Arc<Mutex<Shared>>,
    frame: Option<i32>,
    phase: &'static str,
}

impl PacketLog {
    /// A log and its reader.
    pub fn new() -> (Self, PacketLines) {
        let lines = PacketLines::default();
        (
            Self {
                shared: lines.0.clone(),
                frame: None,
                phase: "start",
            },
            lines,
        )
    }

    fn write(&mut self, kind: &'static str, fields: &str) {
        let frame = self.frame.map_or("null".to_owned(), |f| f.to_string());
        let mut s = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        let seq = s.seq;
        s.seq += 1;
        *s.counts.entry(kind).or_default() += 1;
        s.lines.push(format!(
            "{{\"type\":\"{kind}\"{fields},\"frame\":{frame},\"phase\":\"{}\",\"seq\":{seq}}}",
            self.phase
        ));
    }
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02x}");
    }
    s
}

/// `,"client":C,"size":N,"bytes":"…"`.
fn message(client: ClientId, size: usize, msg: &[u8]) -> String {
    format!(
        ",\"client\":{client},\"size\":{size},\"bytes\":\"{}\"",
        hex(msg)
    )
}

impl PacketObserver for PacketLog {
    fn packet(&mut self, ev: PacketEvent<'_>) {
        let (kind, fields) = match ev {
            PacketEvent::ClientSend { client, msg } => {
                ("client_send", message(client, msg.len(), msg))
            }
            PacketEvent::ClientOut { client, msg } => {
                ("client_out", message(client, msg.len(), msg))
            }
            PacketEvent::Drain => {
                self.phase = "input";
                ("drain", String::new())
            }
            PacketEvent::C2s {
                system,
                client,
                size,
                msg,
            } => (
                if system { "c2s_sys" } else { "c2s" },
                message(client, size, msg),
            ),
            PacketEvent::Dispatch {
                client,
                id,
                size,
                game_frame,
            } => (
                "dispatch",
                format!(
                    ",\"client\":{client},\"game_frame\":{game_frame},\"size\":{size},\"id\":{id}"
                ),
            ),
            PacketEvent::Result { client, code } => (
                "result",
                format!(
                    ",\"client\":{client},\"dispatched\":true,\"code\":{}",
                    code as u8
                ),
            ),
            PacketEvent::Tick { frame } => {
                self.frame = Some(frame);
                self.phase = "tick";
                ("tick", String::new())
            }
            PacketEvent::TickEnd { frame } => {
                self.frame = Some(frame);
                self.phase = "post";
                ("tick_end", String::new())
            }
            PacketEvent::Flush => {
                self.phase = "flush";
                ("flush", String::new())
            }
            PacketEvent::S2c { client, msg } => ("s2c", message(client, msg.len(), msg)),
            PacketEvent::Net {
                client,
                msg,
                direct,
            } => {
                let via = if direct {
                    ",\"via\":\"direct\"".to_owned()
                } else {
                    format!(",\"caller\":\"{FLUSH_CALLER}\"")
                };
                (
                    "net",
                    format!(",\"kind\":1{}{via}", message(client, msg.len(), msg)),
                )
            }
        };
        self.write(kind, &fields);
    }
}
