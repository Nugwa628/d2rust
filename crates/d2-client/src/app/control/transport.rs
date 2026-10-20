//! How command lines reach a controlled game and its replies leave it:
//! the process's own stdin / stdout (`--control stdio`, for a tool that
//! starts the game itself), or a local named pipe
//! `\\.\pipe\d2rust-<pid>` (`--control pipe`, to attach to a game that is
//! already running). Never a network port: the pipe refuses remote
//! clients, and nothing here opens a socket.
//!
//! The transport runs on its own threads and hands the game whole lines
//! through a channel, so a slow or silent driver never stalls a frame.

use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver, Sender};

/// Where the control channel listens (`play --control stdio|pipe`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// The process's stdin and stdout. The game ends when stdin closes.
    Stdio,
    /// A named pipe `\\.\pipe\d2rust-<pid>` (Windows). One client at a
    /// time; when it leaves, the game keeps running and the pipe waits
    /// for the next one.
    Pipe,
}

impl Transport {
    /// `stdio` or `pipe`.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "stdio" => Ok(Transport::Stdio),
            "pipe" => Ok(Transport::Pipe),
            _ => Err(format!("--control {s}: use stdio or pipe")),
        }
    }
}

/// What the transport hands the game, in arrival order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// A driver is there: it gets a `hello` once the game can answer.
    Connected,
    /// One command line.
    Line(String),
    /// The driver left (stdin closed, the pipe client disconnected).
    Closed,
}

/// An open channel: lines in, reply lines out.
pub struct Channel {
    /// Behind a lock only so the channel can live in a Bevy resource;
    /// one system reads it.
    pub incoming: std::sync::Mutex<Receiver<Incoming>>,
    pub replies: Sender<String>,
    /// Where it listens, for the hello line and the log.
    pub name: String,
    /// The game ends when the driver leaves (stdio); a pipe waits for the
    /// next driver instead.
    pub exit_on_close: bool,
}

/// Opens `t`'s channel and starts its threads.
pub fn open(t: Transport) -> std::io::Result<Channel> {
    match t {
        Transport::Stdio => Ok(stdio()),
        Transport::Pipe => pipe(),
    }
}

fn stdio() -> Channel {
    let (tx, incoming) = mpsc::channel();
    let _ = tx.send(Incoming::Connected);
    std::thread::Builder::new()
        .name("control-in".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if tx.send(Incoming::Line(line)).is_err() {
                    return;
                }
            }
            let _ = tx.send(Incoming::Closed);
        })
        .expect("control input thread");
    let (replies, rx) = mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("control-out".into())
        .spawn(move || {
            for reply in rx {
                let mut out = std::io::stdout().lock();
                if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
                    return;
                }
            }
        })
        .expect("control output thread");
    Channel {
        incoming: std::sync::Mutex::new(incoming),
        replies,
        name: "stdio".into(),
        exit_on_close: true,
    }
}

/// The pipe name of this process.
pub fn pipe_name() -> String {
    format!(r"\\.\pipe\d2rust-{}", std::process::id())
}

#[cfg(not(windows))]
fn pipe() -> std::io::Result<Channel> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "--control pipe needs Windows named pipes; use --control stdio",
    ))
}

#[cfg(windows)]
fn pipe() -> std::io::Result<Channel> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    let name = pipe_name();
    let (tx, incoming) = mpsc::channel();
    let (replies, mut rx) = {
        let (s, r) = tokio::sync::mpsc::unbounded_channel::<String>();
        // The game's replies go through a plain channel; this thread
        // forwards them to the pipe task.
        let (game_tx, game_rx) = mpsc::channel::<String>();
        std::thread::Builder::new()
            .name("control-fwd".into())
            .spawn(move || {
                for reply in game_rx {
                    if s.send(reply).is_err() {
                        return;
                    }
                }
            })?;
        (game_tx, r)
    };
    let (ready_tx, ready_rx) = mpsc::channel::<std::io::Result<()>>();
    let pipe = name.clone();
    std::thread::Builder::new()
        .name("control-pipe".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            rt.block_on(async move {
                // Local clients only (the default, set here on purpose).
                let create = |first: bool| -> std::io::Result<NamedPipeServer> {
                    ServerOptions::new()
                        .first_pipe_instance(first)
                        .reject_remote_clients(true)
                        .create(&pipe)
                };
                let mut server = match create(true) {
                    Ok(s) => {
                        let _ = ready_tx.send(Ok(()));
                        s
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                loop {
                    let connected = server.connect().await.is_ok();
                    // The next instance exists before this one closes, so
                    // the name never lapses; a second driver waits on it
                    // until this one leaves.
                    let next = match create(false) {
                        Ok(next) => next,
                        Err(e) => {
                            eprintln!("control: {pipe}: {e}");
                            return;
                        }
                    };
                    if connected {
                        // Replies meant for an earlier driver are dropped.
                        while rx.try_recv().is_ok() {}
                        if tx.send(Incoming::Connected).is_err() {
                            return;
                        }
                        let (r, mut w) = tokio::io::split(server);
                        let mut lines = BufReader::new(r).lines();
                        loop {
                            tokio::select! {
                                line = lines.next_line() => match line {
                                    Ok(Some(l)) => {
                                        if tx.send(Incoming::Line(l)).is_err() {
                                            return;
                                        }
                                    }
                                    _ => break,
                                },
                                reply = rx.recv() => match reply {
                                    Some(s) => {
                                        let line = format!("{s}\n");
                                        if w.write_all(line.as_bytes()).await.is_err() {
                                            break;
                                        }
                                    }
                                    None => return,
                                },
                            }
                        }
                        if tx.send(Incoming::Closed).is_err() {
                            return;
                        }
                    }
                    server = next;
                }
            });
        })?;
    ready_rx
        .recv()
        .map_err(|_| std::io::Error::other("the pipe thread stopped"))??;
    Ok(Channel {
        incoming: std::sync::Mutex::new(incoming),
        replies,
        name,
        exit_on_close: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transports_parse() {
        assert_eq!(Transport::parse("stdio"), Ok(Transport::Stdio));
        assert_eq!(Transport::parse("pipe"), Ok(Transport::Pipe));
        assert!(Transport::parse("tcp").is_err());
        assert!(pipe_name().starts_with(r"\\.\pipe\d2rust-"));
    }
}
