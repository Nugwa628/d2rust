//! Network conditions between a bot and the server: a UDP relay on
//! loopback that delays, jitters and drops datagrams.
//!
//! Each bot gets its own [`Relay`]: the bot connects to the relay's port,
//! the relay forwards to the server from a socket of its own, and replies
//! go back to wherever the bot last sent from (so a bot that moves to a
//! new local socket keeps its connection, as QUIC migration expects).
//! The socket toward the server sits on the bot's own loopback address
//! (127.0.0.2, .3, ...; contracts 10.1), so per-IP limits see one address
//! per bot. Conditions apply in both directions and can change while the
//! bot plays. Loss and jitter come from a seeded generator: the same seed
//! drops the same datagrams of the same traffic.
//!
//! Everything stays on 127.0.0.0/8: no packet leaves the machine.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How the link between one bot and the server behaves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Conditions {
    /// One-way delay added to every datagram, in milliseconds.
    pub latency_ms: u32,
    /// Up to this many milliseconds more, at random per datagram (so
    /// datagrams can arrive out of order).
    pub jitter_ms: u32,
    /// The share of datagrams dropped, 0.0..=1.0, per direction.
    pub loss: f64,
    /// Every datagram is dropped: the link is cut.
    pub cut: bool,
}

impl Conditions {
    /// Applies `KEY=VALUE` settings (`latency`, `jitter` in ms, `loss` in
    /// percent, `cut=on|off`).
    pub fn apply(&mut self, settings: &[(String, String)]) -> Result<(), String> {
        for (k, v) in settings {
            let bad = || format!("net {k}={v}: not a valid value");
            match k.as_str() {
                "latency" => self.latency_ms = ms(v).ok_or_else(bad)?,
                "jitter" => self.jitter_ms = ms(v).ok_or_else(bad)?,
                "loss" => {
                    let p: f64 = v.trim_end_matches('%').parse().map_err(|_| bad())?;
                    if !(0.0..=100.0).contains(&p) {
                        return Err(bad());
                    }
                    self.loss = p / 100.0;
                }
                "cut" => {
                    self.cut = match v.as_str() {
                        "on" => true,
                        "off" => false,
                        _ => return Err(bad()),
                    }
                }
                _ => return Err(format!("net {k}: unknown (latency, jitter, loss, cut)")),
            }
        }
        Ok(())
    }
}

/// Milliseconds from `80` or `80ms`.
fn ms(v: &str) -> Option<u32> {
    v.trim_end_matches("ms").parse().ok()
}

/// A small seeded generator (splitmix64): reproducible loss and jitter.
#[derive(Debug)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in 0.0..1.0.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Which way a datagram goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dir {
    /// Bot to server.
    Up,
    /// Server to bot.
    Down,
}

/// A datagram waiting for its delivery time.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Due {
    at: Instant,
    seq: u64,
    dir: Dir,
    bytes: Vec<u8>,
}

/// What the relay's threads share.
#[derive(Debug)]
struct Shared {
    cond: Mutex<Conditions>,
    rng: Mutex<Rng>,
    /// The bot's address, as last seen.
    bot: Mutex<Option<SocketAddr>>,
    /// Where the server listens now (it moves on a restart).
    server: Mutex<SocketAddr>,
    queue: Mutex<BinaryHeap<Reverse<Due>>>,
    wake: Condvar,
    stop: AtomicBool,
    seq: AtomicU64,
    /// Datagrams relayed and dropped, for the report.
    relayed: AtomicU64,
    dropped: AtomicU64,
}

/// One bot's relay.
#[derive(Debug)]
pub struct Relay {
    shared: Arc<Shared>,
    addr: SocketAddr,
    threads: Vec<JoinHandle<()>>,
}

/// How long a reader blocks before it looks at the stop flag.
const POLL: Duration = Duration::from_millis(50);

impl Relay {
    /// A relay on a free loopback port that forwards to `server` from
    /// `local`. Each bot gets its own address in 127.0.0.0/8 (contracts
    /// 10.1), so the server's per-IP limits see one address per bot; a
    /// scenario that tests a per-IP limit puts its bots on one address.
    pub fn start(server: SocketAddr, seed: u64, local: IpAddr) -> std::io::Result<Self> {
        let front = UdpSocket::bind("127.0.0.1:0")?;
        let back = UdpSocket::bind(SocketAddr::new(local, 0))?;
        front.set_read_timeout(Some(POLL))?;
        back.set_read_timeout(Some(POLL))?;
        let addr = front.local_addr()?;
        let shared = Arc::new(Shared {
            cond: Mutex::new(Conditions::default()),
            rng: Mutex::new(Rng(seed)),
            bot: Mutex::new(None),
            server: Mutex::new(server),
            queue: Mutex::new(BinaryHeap::new()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            relayed: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        });
        let mut threads = Vec::new();
        {
            let (s, f) = (shared.clone(), front.try_clone()?);
            threads.push(std::thread::spawn(move || read_loop(&s, &f, Dir::Up)));
        }
        {
            let (s, b) = (shared.clone(), back.try_clone()?);
            threads.push(std::thread::spawn(move || read_loop(&s, &b, Dir::Down)));
        }
        {
            let s = shared.clone();
            threads.push(std::thread::spawn(move || send_loop(&s, &front, &back)));
        }
        Ok(Self {
            shared,
            addr,
            threads,
        })
    }

    /// The address a bot connects to.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The conditions now.
    pub fn conditions(&self) -> Conditions {
        *self.shared.cond.lock().expect("conditions")
    }

    /// Changes the conditions; datagrams already on their way keep theirs.
    pub fn set(&self, c: Conditions) {
        *self.shared.cond.lock().expect("conditions") = c;
    }

    /// Points the relay at a server that moved (a restart on a new port).
    pub fn retarget(&self, server: SocketAddr) {
        *self.shared.server.lock().expect("server") = server;
    }

    /// Datagrams relayed and dropped so far.
    pub fn counts(&self) -> (u64, u64) {
        (
            self.shared.relayed.load(Ordering::Relaxed),
            self.shared.dropped.load(Ordering::Relaxed),
        )
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Reads one side and queues each datagram for its delivery time, or
/// drops it.
fn read_loop(s: &Shared, sock: &UdpSocket, dir: Dir) {
    let mut buf = vec![0u8; 65536];
    while !s.stop.load(Ordering::Relaxed) {
        let (n, from) = match sock.recv_from(&mut buf) {
            Ok(x) => x,
            Err(_) => continue,
        };
        if dir == Dir::Up {
            *s.bot.lock().expect("bot") = Some(from);
        }
        let c = *s.cond.lock().expect("conditions");
        let (lost, extra) = {
            let mut rng = s.rng.lock().expect("rng");
            let lost = c.cut || (c.loss > 0.0 && rng.unit() < c.loss);
            let extra = if c.jitter_ms > 0 {
                (rng.unit() * f64::from(c.jitter_ms)) as u64
            } else {
                0
            };
            (lost, extra)
        };
        if lost {
            s.dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let at = Instant::now() + Duration::from_millis(u64::from(c.latency_ms) + extra);
        let seq = s.seq.fetch_add(1, Ordering::Relaxed);
        s.queue.lock().expect("queue").push(Reverse(Due {
            at,
            seq,
            dir,
            bytes: buf[..n].to_vec(),
        }));
        s.wake.notify_all();
    }
}

/// Sends each queued datagram when its time comes.
fn send_loop(s: &Shared, front: &UdpSocket, back: &UdpSocket) {
    let mut q = s.queue.lock().expect("queue");
    while !s.stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        match q.peek() {
            Some(Reverse(d)) if d.at <= now => {
                let Reverse(d) = q.pop().expect("peeked");
                drop(q);
                let sent = match d.dir {
                    Dir::Up => {
                        let to = *s.server.lock().expect("server");
                        back.send_to(&d.bytes, to)
                    }
                    Dir::Down => match *s.bot.lock().expect("bot") {
                        Some(to) => front.send_to(&d.bytes, to),
                        None => Ok(0),
                    },
                };
                if sent.is_ok() {
                    s.relayed.fetch_add(1, Ordering::Relaxed);
                }
                q = s.queue.lock().expect("queue");
            }
            Some(Reverse(d)) => {
                let wait = d.at - now;
                q = s.wake.wait_timeout(q, wait).expect("queue").0;
            }
            None => q = s.wake.wait_timeout(q, POLL).expect("queue").0,
        }
    }
}
