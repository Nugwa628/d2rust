//! `d2rust-server --probe`: a deep check of the game data on the server's
//! own machine. One game is built and a new character of every class joins
//! it over in-memory game streams, plays until it is in the game, and
//! leaves; each character must then be saved. It touches every file a game
//! needs from the game data, so it also finds the files the server reads
//! (`tools/server/game-data.sh`).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use d2_net::{pair, Lane, MemoryEnd, NetError, Packet, Transport};
use d2_server::game::{create_request_for, new_character, GameData, CLASS_NAMES};
use tokio::sync::mpsc;

use super::{AttachedStream, GameEvent, GameSpec, Games, GamesConfig, Seat};
use crate::accounts::{AccountId, NewChar};
use crate::log::ServerLog;

/// How long a probe waits for every character to get in and be saved.
const PROBE_WAIT: Duration = Duration::from_secs(60);

/// The server end of an in-memory game stream; the welcome is kept.
struct ProbeStream {
    end: MemoryEnd,
    welcome: Arc<Mutex<Option<Vec<u8>>>>,
}

impl Transport for ProbeStream {
    fn send(&mut self, p: Packet) -> Result<(), NetError> {
        self.end.send(p)
    }
    fn recv(&mut self) -> Result<Option<Packet>, NetError> {
        self.end.recv()
    }
}

impl AttachedStream for ProbeStream {
    fn send_body(&mut self, body: Vec<u8>) -> Result<(), NetError> {
        *self.welcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(body);
        Ok(())
    }
}

/// Runs the probe with the game data `data`, writing its characters under
/// `dir`. Returns what it did, or why it failed.
pub fn probe(
    data: GameData,
    cfg: GamesConfig,
    dir: &Path,
    log: Arc<ServerLog>,
) -> Result<String, String> {
    let started = Instant::now();
    let games = Games::new(cfg, data, log);
    let (tx, mut rx) = mpsc::channel(1024);
    let spec = GameSpec {
        id: 1,
        name: "probe".into(),
        difficulty: 0,
        hardcore: false,
        expansion: true,
        max_players: CLASS_NAMES.len() as u8,
    };
    let game = games.start(spec, tx).map_err(|e| format!("start: {e}"))?;
    let mut ends = Vec::new();
    for (class, class_name) in CLASS_NAMES.iter().enumerate() {
        let name = format!("Probe{}", (b'a' + class as u8) as char);
        let seat = Seat {
            account: AccountId(1),
            character: name.clone(),
            file: dir.join(format!("{name}.d2rc")),
            new: Some(NewChar {
                name: name.clone(),
                class: class as u8,
                hardcore: false,
                expansion: true,
                progression: 0,
            }),
        };
        let (mut end, server_end) = pair();
        let welcome = Arc::new(Mutex::new(None));
        game.admit(
            seat,
            ProbeStream {
                end: server_end,
                welcome,
            },
        )
        .map_err(|e| format!("{name}: admit: {e}"))?;
        let req = create_request_for(
            &new_character(class_name, &name).map_err(|e| e.to_string())?,
            800,
            600,
        );
        end.send(Packet::Send {
            lane: Lane::System,
            msg: req.encode().to_vec(),
        })
        .map_err(|e| e.to_string())?;
        ends.push((name, end, false, false));
    }
    // Every player: the 0x02 answered with the 0x6B, then (once the game's
    // 0x04 says the room is ready) the leave.
    let until = Instant::now() + PROBE_WAIT;
    while ends.iter().any(|(_, _, _, left)| !left) {
        if Instant::now() > until {
            let stuck: Vec<&str> = ends.iter().filter(|e| !e.3).map(|e| e.0.as_str()).collect();
            return Err(format!("not in the game in time: {}", stuck.join(", ")));
        }
        for (_, end, joined, left) in ends.iter_mut() {
            while let Ok(Some(p)) = end.recv() {
                let Packet::Deliver(m) = p else { continue };
                match m.first() {
                    Some(0x02) if !*joined => {
                        *joined = true;
                        let _ = end.send(Packet::Send {
                            lane: Lane::System,
                            msg: vec![0x6B],
                        });
                    }
                    Some(0x04) if *joined && !*left => {
                        *left = true;
                        let _ = end.send(Packet::Send {
                            lane: Lane::System,
                            msg: vec![0x69],
                        });
                    }
                    _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // Each leave saves; the last one closes the game.
    let mut saved = 0;
    let until = Instant::now() + PROBE_WAIT;
    loop {
        match rx.try_recv() {
            Ok(GameEvent::Saved { .. }) => saved += 1,
            Ok(GameEvent::Closed { .. }) => break,
            Ok(_) => {}
            Err(_) if Instant::now() > until => return Err("the game did not close".into()),
            Err(_) => {
                drop(std::mem::take(&mut ends));
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    let files = CLASS_NAMES
        .iter()
        .enumerate()
        .filter(|(c, _)| {
            dir.join(format!("Probe{}.d2rc", (b'a' + *c as u8) as char))
                .is_file()
        })
        .count();
    if files != CLASS_NAMES.len() {
        return Err(format!(
            "{files} of {} characters were saved",
            CLASS_NAMES.len()
        ));
    }
    Ok(format!(
        "probe: one game, {} characters joined and were saved ({saved} saves) in {} ms",
        CLASS_NAMES.len(),
        started.elapsed().as_millis()
    ))
}
