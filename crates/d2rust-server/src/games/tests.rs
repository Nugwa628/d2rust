//! The games on the user's game data: players join over in-memory game
//! streams, play, leave and come back; the game's rules about seats,
//! saves and closing.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use d2_net::msg::ErrorCode;
use d2_net::wire::Welcome;
use d2_net::{pair, Lane, MemoryEnd, NetError, Packet, Transport};
use d2_server::game::{create_request_for, new_character, GameData};
use d2_server::seams::ClientId;
use d2_sim::units::lists::client_state;
use tokio::sync::mpsc;

use super::*;
use crate::accounts::{AccountId, NewChar};
use crate::log::ServerLog;

/// The user's game data, loaded once for every test.
fn data() -> GameData {
    static DATA: OnceLock<GameData> = OnceLock::new();
    DATA.get_or_init(|| {
        let dir = std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR");
        GameData::select(Some(Path::new(&dir))).expect("the game data loads")
    })
    .clone()
}

/// A temporary server folder, removed at the end.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "d2rust-games-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Folder(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // A failed test shows the server's log.
        if std::thread::panicking() {
            let log = std::fs::read_to_string(self.0.join("logs").join(crate::log::FILE_NAME));
            eprintln!("server log:\n{}", log.unwrap_or_default());
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config() -> GamesConfig {
    GamesConfig {
        max_games: 4,
        max_players: 8,
        grace: Duration::from_secs(2),
        frame_pause: Duration::from_millis(1),
        save_interval: Duration::from_secs(3600),
        unused_close: Duration::from_millis(200),
        dev_test: false,
    }
}

struct Server {
    games: Games,
    events: mpsc::Receiver<GameEvent>,
    tx: mpsc::Sender<GameEvent>,
    dir: Folder,
}

impl Server {
    fn new(name: &str) -> Self {
        Self::with(name, config())
    }

    fn with(name: &str, cfg: GamesConfig) -> Self {
        let dir = Folder::new(name);
        let log = Arc::new(ServerLog::open(&dir.0.join("logs")).unwrap());
        let (tx, events) = mpsc::channel(1024);
        Server {
            games: Games::new(cfg, data(), log),
            events,
            tx,
            dir,
        }
    }

    fn start(&self, id: u32, difficulty: u8) -> GameHandle {
        self.games
            .start(
                GameSpec {
                    id,
                    name: format!("game{id}"),
                    difficulty,
                    hardcore: false,
                    expansion: true,
                    max_players: 0,
                },
                self.tx.clone(),
            )
            .unwrap()
    }

    fn seat(&self, account: i64, name: &str, new_class: Option<u8>) -> Seat {
        self.seat_with(account, name, new_class, 0)
    }

    fn seat_with(&self, account: i64, name: &str, new_class: Option<u8>, progression: u8) -> Seat {
        Seat {
            account: AccountId(account),
            character: name.into(),
            file: self
                .dir
                .0
                .join("data/characters")
                .join(account.to_string())
                .join(format!("{name}.d2rc")),
            new: new_class.map(|class| NewChar {
                name: name.into(),
                class,
                hardcore: false,
                expansion: true,
                progression,
            }),
        }
    }

    /// Every event so far.
    fn drain(&mut self) -> Vec<GameEvent> {
        let mut out = Vec::new();
        while let Ok(e) = self.events.try_recv() {
            out.push(e);
        }
        out
    }

    /// Waits for an event `want` picks (others are kept in `seen`).
    fn wait_event(
        &mut self,
        seen: &mut Vec<GameEvent>,
        want: impl Fn(&GameEvent) -> bool,
    ) -> GameEvent {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(i) = seen.iter().position(&want) {
                return seen.remove(i);
            }
            seen.extend(self.drain());
            assert!(Instant::now() < until, "no such event; saw {seen:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// The server's end of a test player's game stream: an in-memory pair end
/// that hands the game's welcome to the player's side.
struct TestStream {
    end: MemoryEnd,
    welcome: Arc<Mutex<Option<Vec<u8>>>>,
}

impl Transport for TestStream {
    fn send(&mut self, p: Packet) -> Result<(), NetError> {
        self.end.send(p)
    }
    fn recv(&mut self) -> Result<Option<Packet>, NetError> {
        self.end.recv()
    }
}

impl AttachedStream for TestStream {
    fn send_body(&mut self, body: Vec<u8>) -> Result<(), NetError> {
        *self.welcome.lock().unwrap() = Some(body);
        Ok(())
    }
}

/// A test player: its end of the game stream, acting as the game client
/// does at the start of a game (0x67, then 0x6B after the server's 0x02).
struct Player {
    end: MemoryEnd,
    welcome: Welcome,
    joined: bool,
    got: Vec<Vec<u8>>,
}

impl Player {
    /// Admits `seat` into `game` and sends the 0x67 join of `name`
    /// (`class` for the request; the server takes the seat's).
    fn join(
        game: &GameHandle,
        seat: Seat,
        name: &str,
        difficulty: u8,
    ) -> Result<Player, ErrorCode> {
        let (end, server_end) = pair();
        let welcome = Arc::new(Mutex::new(None));
        game.admit(
            seat,
            TestStream {
                end: server_end,
                welcome: welcome.clone(),
            },
        )?;
        let body = welcome.lock().unwrap().take().expect("a welcome");
        let welcome = d2_net::wire::parse_answer(&body).expect("a game welcome");
        let mut p = Player {
            end,
            welcome,
            joined: false,
            got: Vec::new(),
        };
        let mut req = create_request_for(&new_character("barbarian", name).unwrap(), 800, 600);
        req.difficulty = difficulty;
        p.send(Lane::System, req.encode().to_vec());
        Ok(p)
    }

    fn send(&mut self, lane: Lane, msg: Vec<u8>) {
        let _ = self.end.send(Packet::Send { lane, msg });
    }

    /// Reads what arrived; answers the 0x02 with the 0x6B.
    fn pump(&mut self) -> Result<(), NetError> {
        while let Some(p) = self.end.recv()? {
            if let Packet::Deliver(m) = p {
                if m.first() == Some(&0x02) && !self.joined {
                    self.joined = true;
                    self.send(Lane::System, vec![0x6B]);
                }
                self.got.push(m);
            }
        }
        Ok(())
    }

    fn client(&self) -> ClientId {
        self.welcome.client
    }
}

/// A client's state in the game's client list, if it has a record.
fn state(game: &GameHandle, client: ClientId) -> Option<u32> {
    game.with_thread(move |i| {
        let s = &i.hub.host.game;
        let id = s.sim_client(client)?;
        s.game.lists.client(id).map(|e| e.state)
    })
    .unwrap()
}

/// Pumps the players until every one is in the game.
fn until_in_game(game: &GameHandle, players: &mut [&mut Player]) {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        for p in players.iter_mut() {
            p.pump().unwrap();
        }
        if players
            .iter()
            .all(|p| state(game, p.client()) == Some(client_state::IN_GAME))
        {
            return;
        }
        assert!(Instant::now() < until, "not every player got into the game");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn read_file_record(path: &Path, name: &str) -> Vec<u8> {
    crate::characters::decode(name, &std::fs::read(path).unwrap()).unwrap()
}

/// The join's character-state messages (the same set the games' re-entry
/// keeps): stats, items, inventory relator, quests/waypoints, mercenary,
/// party, skills and the vitals sync.
fn static_state(m: &[u8]) -> bool {
    matches!(
        m.first(),
        Some(
            0x1D | 0x1E
                | 0x1F
                | 0x21
                | 0x22
                | 0x23
                | 0x3E
                | 0x48
                | 0x28
                | 0x29
                | 0x5E
                | 0x89
                | 0x5F
                | 0x7A
                | 0x81
                | 0x65
                | 0x8D
                | 0x7B
                | 0x94
                | 0x95
                | 0x9C
                | 0x9D
        )
    )
}

const GOLD: u16 = 14;

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn two_players_play_one_game_with_their_own_characters() {
    let s = Server::new("two");
    let g = s.start(1, 0);
    let mut a = Player::join(&g, s.seat(1, "Alpha", Some(4)), "Alpha", 0).unwrap();
    let mut b = Player::join(&g, s.seat(2, "Beta", Some(1)), "Beta", 0).unwrap();
    assert_ne!(a.client(), b.client());
    until_in_game(&g, &mut [&mut a, &mut b]);
    let (pa, pb, ca, cb) = g
        .with_thread({
            let (ca, cb) = (a.client(), b.client());
            move |i| {
                let s = &i.hub.host.game;
                let (pa, pb) = (s.player_of(ca).unwrap(), s.player_of(cb).unwrap());
                let class = |p| s.events.action.sys.units.get(p).map(|u| u.class);
                (pa, pb, class(pa), class(pb))
            }
        })
        .unwrap();
    assert_ne!(pa, pb, "one player unit each");
    // The seat's class, not the request's (both asked for a barbarian).
    assert_eq!((ca, cb), (Some(4), Some(1)));
    assert_eq!(g.players(), 2);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn leaving_saves_the_character_and_a_new_game_loads_it() {
    let mut s = Server::new("persist");
    let g = s.start(1, 0);
    let seat = s.seat(7, "Keeper", Some(0));
    let file = seat.file.clone();
    let mut p = Player::join(&g, seat, "Keeper", 0).unwrap();
    until_in_game(&g, &mut [&mut p]);
    let client = p.client();
    g.with_thread(move |i| {
        let sim = &mut i.hub.host.game;
        let player = sim.player_of(client).unwrap();
        sim.events
            .action
            .with(&mut sim.game, |_, v| v.set_base(player, GOLD, 1234));
    })
    .unwrap();
    // Save and Exit: C→S 0x69, then the stream ends.
    p.send(Lane::System, vec![0x69]);
    let mut seen = Vec::new();
    let saved = s.wait_event(&mut seen, |e| matches!(e, GameEvent::Saved { .. }));
    let GameEvent::Saved {
        account, summary, ..
    } = saved
    else {
        unreachable!()
    };
    assert_eq!(account, AccountId(7));
    assert_eq!(summary.name, "Keeper");
    assert_eq!(summary.class, 0);
    assert_eq!(summary.level, 1);
    assert_eq!(summary.difficulties, 1, "a new character has Normal only");
    assert_eq!(summary.look.len(), 32);
    drop(p);
    s.wait_event(
        &mut seen,
        |e| matches!(e, GameEvent::Left { character, .. } if character == "Keeper"),
    );
    s.wait_event(&mut seen, |e| {
        matches!(e, GameEvent::Closed { game: 1, .. })
    });
    assert!(g.is_closed());
    assert!(!read_file_record(&file, "Keeper").is_empty());

    // A new game (as after a server restart) loads the file: the gold is back.
    let g2 = s.start(2, 0);
    let mut p2 = Player::join(&g2, s.seat(7, "Keeper", None), "Keeper", 0).unwrap();
    until_in_game(&g2, &mut [&mut p2]);
    let client = p2.client();
    let gold = g2
        .with_thread(move |i| {
            let sim = &mut i.hub.host.game;
            let player = sim.player_of(client).unwrap();
            let class = sim.events.action.sys.units.get(player).map(|u| u.class);
            let gold = sim
                .events
                .action
                .with(&mut sim.game, |_, v| v.stats.unit_base(player, GOLD, 0));
            (class, gold)
        })
        .unwrap();
    assert_eq!(gold, (Some(0), 1234));
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_lost_player_re_enters_with_its_position_and_sees_the_other_player() {
    let mut s = Server::new("resume");
    let g = s.start(1, 0);
    let mut a = Player::join(&g, s.seat(1, "Alpha", Some(4)), "Alpha", 0).unwrap();
    let mut b = Player::join(&g, s.seat(2, "Beta", Some(1)), "Beta", 0).unwrap();
    until_in_game(&g, &mut [&mut a, &mut b]);
    // The load completes on a later frame; the join's character state is
    // in that same flush.
    let until = Instant::now() + Duration::from_secs(20);
    while !a.got.iter().any(|m| m.first() == Some(&0x04)) {
        a.pump().unwrap();
        b.pump().unwrap();
        assert!(
            Instant::now() < until,
            "no LoadComplete for the joiner: {:02X?}",
            a.got
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let (ca, cb) = (a.client(), b.client());
    let old_token = a.welcome.resume;
    let join_messages: Vec<Vec<u8>> = a.got.iter().filter(|m| static_state(m)).cloned().collect();
    assert!(
        !join_messages.is_empty(),
        "the join sent character state: {:02X?}",
        a.got
    );
    let guid_of = |client| {
        g.with_thread(move |i| {
            let sim = &i.hub.host.game;
            let p = sim.player_of(client).unwrap();
            sim.events.action.sys.units.get(p).map(|u| u.guid)
        })
        .unwrap()
        .expect("a player unit")
    };
    let (ga, gb) = (guid_of(ca), guid_of(cb));
    let position = || {
        g.with_thread(move |i| {
            let sim = &mut i.hub.host.game;
            let p = sim.player_of(ca).unwrap();
            sim.events.action.hooks().path_position(p)
        })
        .unwrap()
    };
    let (x, y) = position();

    // The connection is lost: the game parks the slot (grace 2 s).
    drop(a);
    for _ in 0..40 {
        b.pump().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }

    // The resume is a re-entry: the same client, a new token, and the game
    // state again (rooms, the player at its position, the other player).
    let (end, server_end) = pair();
    let body = Arc::new(Mutex::new(None));
    g.resume(
        old_token,
        TestStream {
            end: server_end,
            welcome: body.clone(),
        },
    )
    .unwrap();
    let body = body.lock().unwrap().take().expect("a resume welcome");
    let welcome = d2_net::wire::parse_answer(&body).expect("a game welcome");
    assert_eq!(welcome.client, ca, "the same client id");
    assert_ne!(welcome.resume, old_token, "a new resume token");
    let mut r = Player {
        end,
        welcome,
        joined: true,
        got: Vec::new(),
    };
    let guid_at = |m: &[u8]| u32::from_le_bytes([m[1], m[2], m[3], m[4]]);
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        r.pump().unwrap();
        let own = r
            .got
            .iter()
            .find(|m| m.first() == Some(&0x59) && guid_at(m) == ga);
        let other = r
            .got
            .iter()
            .any(|m| m.first() == Some(&0x59) && guid_at(m) == gb);
        if own.is_some() && other && r.got.iter().any(|m| m.first() == Some(&0x07)) {
            break;
        }
        assert!(
            Instant::now() < until,
            "the re-entry state did not arrive: {:02X?}",
            r.got
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // The join's character state is sent again (contracts v12 3.7): every
    // such message the join sent arrives on the resumed stream too.
    let resumed: Vec<&Vec<u8>> = r.got.iter().filter(|m| static_state(m)).collect();
    for m in &join_messages {
        assert!(
            resumed.contains(&m),
            "the re-entry did not resend {m:02X?}; it sent {resumed:02X?}"
        );
    }
    // Nothing was reloaded or saved: the game still holds both players at
    // their positions, and the old token is dead.
    assert_eq!(g.players(), 2);
    assert_eq!(position(), (x, y), "the player was not moved");
    assert!(
        !s.drain()
            .iter()
            .any(|e| matches!(e, GameEvent::Saved { .. })),
        "a resume saves nothing"
    );
    let (_late, server_end) = pair();
    let refused = Arc::new(Mutex::new(None));
    assert_eq!(
        g.resume(
            old_token,
            TestStream {
                end: server_end,
                welcome: refused.clone(),
            }
        )
        .err(),
        Some(ErrorCode::UNKNOWN_RESUME),
        "the old token stops working"
    );
}

/// The state messages of a client's character, as the client applies them:
/// the last value of each replaceable record. GUIDs differ between games,
/// so they are zeroed before two views are compared.
#[derive(Debug, Default, PartialEq, Eq)]
struct CharacterView {
    stats: std::collections::BTreeMap<u8, u32>,
    /// Items in a grid, the body, the belt or the cursor (never the
    /// ground), one normalized last record each, in a GUID-independent
    /// order.
    items: Vec<Vec<u8>>,
    /// 0x21 / 0x22 item-skill links (join only).
    item_skills: Vec<Vec<u8>>,
    base_skills: Option<Vec<u8>>,
    hands: std::collections::BTreeMap<u8, Vec<u8>>,
    hotkeys: std::collections::BTreeMap<u8, Vec<u8>>,
    portal: Option<u32>,
    quest_availability: Option<Vec<u8>>,
    quest_info: Vec<Vec<u8>>,
    game_quest: Option<Vec<u8>>,
    quest_status: std::collections::BTreeMap<u8, Vec<u8>>,
    quest_log: Option<Vec<u8>>,
    mercs: Vec<Vec<u8>>,
}

fn u16_at(m: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([m[at], m[at + 1]])
}

fn u32_at(m: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([m[at], m[at + 1], m[at + 2], m[at + 3]])
}

/// `m` with the 4-byte GUIDs at `ranges` zeroed.
fn without_guids(m: &[u8], ranges: &[(usize, usize)]) -> Vec<u8> {
    let mut m = m.to_vec();
    for &(at, end) in ranges {
        m[at..end].fill(0);
    }
    m
}

/// The item flags the move queue keeps on a live item and a save's load
/// normalizes away (the update list's bits and `picked to belt`). They are
/// not part of the item's view.
const QUEUED_ITEM_FLAGS: u32 = 0x800 | 0x1000 | 0x2000 | 0x10000 | 0x200000;

/// `m` with the GUIDs zeroed and the item header's queued flags masked;
/// `head` is the item bit stream's offset in the message.
fn item_record(m: &[u8], head: usize, owned: bool) -> Vec<u8> {
    let mut m = without_guids(m, if owned { &[(4, 8), (9, 13)] } else { &[(4, 8)] });
    let flags = u32_at(&m, head) & !QUEUED_ITEM_FLAGS;
    m[head..head + 4].copy_from_slice(&flags.to_le_bytes());
    m
}

/// The first `n` bits of an LSB-first item bit stream.
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: usize) -> Option<u32> {
        if self.at + n > self.bytes.len() * 8 {
            return None;
        }
        let mut v = 0;
        for i in 0..n {
            let at = self.at + i;
            v |= u32::from((self.bytes[at / 8] >> (at % 8)) & 1) << i;
        }
        self.at += n;
        Some(v)
    }
}

/// The item header's mode (0 grid, 1 body, 2 belt, 3 ground, 4 cursor,
/// 5 dropping).
fn item_mode(stream: &[u8]) -> Option<u8> {
    let mut b = Bits {
        bytes: stream,
        at: 0,
    };
    b.read(32)?;
    b.read(10)?;
    Some(b.read(3)? as u8)
}

/// The name of a 0x59 add message, or empty.
fn add_name(m: &[u8]) -> &[u8] {
    let n = m[6..22].iter().position(|&b| b == 0).unwrap_or(16);
    &m[6..6 + n]
}

/// The character state a client is told, from its S→C messages.
fn character_view(messages: &[Vec<u8>], name: &str) -> CharacterView {
    let me = messages
        .iter()
        .find(|m| m.first() == Some(&0x59) && add_name(m) == name.as_bytes())
        .map(|m| u32_at(m, 1))
        .expect("the client's own player add");
    let mut v = CharacterView::default();
    let mut items: std::collections::BTreeMap<u32, Vec<u8>> = Default::default();
    let mut info: std::collections::BTreeMap<(u8, u32), Vec<u8>> = Default::default();
    let mut mercs: std::collections::BTreeMap<u32, Vec<u8>> = Default::default();
    let mut item_skills = Vec::new();
    for m in messages {
        let Some(&id) = m.first() else { continue };
        match id {
            0x1D if m.len() == 3 => {
                v.stats.insert(m[1], u32::from(m[2]));
            }
            0x1E if m.len() == 4 => {
                v.stats.insert(m[1], u32::from(u16_at(m, 2)));
            }
            0x1F if m.len() == 6 => {
                v.stats.insert(m[1], u32_at(m, 2));
            }
            0x9C if m.len() >= 8 && usize::from(m[2]) == m.len() => {
                let mode = item_mode(&m[8..]);
                if mode.is_some_and(|mode| !matches!(mode, 3 | 5)) {
                    items.insert(u32_at(m, 4), item_record(m, 8, false));
                }
            }
            0x9D if m.len() >= 13 && usize::from(m[2]) == m.len() => {
                let owner = (m[8], u32_at(m, 9));
                let mode = item_mode(&m[13..]);
                if owner == (0, me) && mode.is_some_and(|mode| !matches!(mode, 3 | 5)) {
                    items.insert(u32_at(m, 4), item_record(m, 13, true));
                }
            }
            0x21 if m.len() == 12 => item_skills.push(without_guids(m, &[(3, 7)])),
            0x22 if m.len() == 12 => item_skills.push(without_guids(m, &[(3, 7)])),
            0x28 if m.len() == 103 => {
                info.insert((m[1], u32_at(m, 2)), without_guids(m, &[(2, 6)]));
            }
            0x29 if m.len() == 97 => v.game_quest = Some(m.clone()),
            0x5D if m.len() == 6 => {
                v.quest_status.insert(m[1], m.clone());
            }
            0x5E if m.len() == 38 => v.quest_availability = Some(m.clone()),
            0x5F if m.len() == 5 => v.portal = Some(u32_at(m, 1)),
            0x52 if m.len() == 42 => v.quest_log = Some(m.clone()),
            0x23 if m.len() == 13 => {
                v.hands.insert(m[6], without_guids(m, &[(2, 6), (9, 13)]));
            }
            0x7B if m.len() == 8 => {
                v.hotkeys.insert(m[1], without_guids(m, &[(4, 8)]));
            }
            0x94 if m.len() >= 6 => {
                v.base_skills = Some(without_guids(m, &[(2, 6)]));
            }
            // The mercenary add (0x81) and its pet action (0x7A).
            0x81 if m.len() == 20 => {
                mercs.insert(u32_at(m, 8), without_guids(m, &[(4, 8), (8, 12)]));
            }
            0x7A if m.len() == 13 => {
                let pet = u32_at(m, 9);
                if m[1] == 0 {
                    mercs.remove(&pet);
                } else {
                    mercs.insert(pet, without_guids(m, &[(5, 9), (9, 13)]));
                }
            }
            _ => {}
        }
    }
    v.items = items.into_values().collect();
    v.items.sort();
    v.quest_info = info.into_values().collect();
    v.quest_info.sort();
    v.item_skills = item_skills;
    v.item_skills.sort();
    v.mercs = mercs.into_values().collect();
    v.mercs.sort();
    v
}

/// A resume after the state changed: the joined client gains a level (the
/// game's own level-up: stat messages, party info, the item relink), then
/// drops and resumes. The client's view must match a fresh join of the
/// same character from the saved file, so a replay of the join-time state
/// (contracts v12 3.7) is caught.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_resume_after_the_state_changed_matches_a_fresh_join() {
    let mut s = Server::new("resume-state");
    let g = s.start(1, 0);
    let seat = s.seat(1, "Climber", Some(4));
    let mut p = Player::join(&g, seat, "Climber", 0).unwrap();
    until_in_game(&g, &mut [&mut p]);
    let client = p.client();
    // Level 2 through the game's own experience path: the tick sends the
    // changed stats and the level-up's messages.
    g.with_thread(move |i| {
        let sim = &mut i.hub.host.game;
        let player = sim.player_of(client).unwrap();
        let t = sim
            .events
            .action
            .sys
            .hooks
            .vitals
            .clone()
            .expect("the vitals tables");
        let class = sim
            .events
            .action
            .sys
            .units
            .get(player)
            .map_or(0, |r| r.class as i32);
        let gain = t.threshold_in(&d2_sim::rules::ORIGINAL, class, 2);
        let s = &mut sim.events.action.sys;
        let v = d2_sim::wiring::action::View::of(&mut s.units, &mut s.stats, &s.data, &mut s.hooks);
        let mut cv = d2_sim::wiring::action::combat::CombatView {
            game: &mut sim.game,
            v,
        };
        d2_sim::combat::vitals::add_experience(&mut cv, &t, player, gain);
    })
    .unwrap();
    // The level's stat message reaches the client.
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        p.pump().unwrap();
        let leveled = p.got.iter().any(|m| {
            m.get(1) == Some(&12)
                && m.first().is_some_and(|&id| matches!(id, 0x1D..=0x1F))
                && u32::from(m.last().copied().unwrap_or(0)) >= 2
        });
        if leveled {
            break;
        }
        assert!(
            Instant::now() < until,
            "no level-up stat message: {:02X?}",
            p.got
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let level = g
        .with_thread(move |i| {
            let sim = &mut i.hub.host.game;
            let player = sim.player_of(client).unwrap();
            sim.events.action.sys.stats.unit_base(player, 12, 0)
        })
        .unwrap();
    assert!(level > 1, "the character leveled: {level}");
    // The saved character has the new level: the fresh join loads it.
    let results = g
        .with_thread(move |i| i.hub.host.game.save_characters())
        .unwrap();
    assert!(results.iter().all(|(_, r)| r.is_ok()), "{results:?}");
    let mut seen = Vec::new();
    s.wait_event(&mut seen, |e| matches!(e, GameEvent::Saved { .. }));

    // The connection is lost, then the client comes back.
    let old_token = p.welcome.resume;
    drop(p);
    std::thread::sleep(Duration::from_millis(100));
    let (end, server_end) = pair();
    let body = Arc::new(Mutex::new(None));
    g.resume(
        old_token,
        TestStream {
            end: server_end,
            welcome: body.clone(),
        },
    )
    .unwrap();
    let body = body.lock().unwrap().take().expect("a resume welcome");
    let welcome = d2_net::wire::parse_answer(&body).expect("a game welcome");
    let mut resumed = Player {
        end,
        welcome,
        joined: true,
        got: Vec::new(),
    };
    for _ in 0..80 {
        resumed.pump().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }

    // A fresh join of the same character from the file just saved.
    let g2 = s.start(2, 0);
    let mut fresh = Player::join(&g2, s.seat(1, "Climber", None), "Climber", 0).unwrap();
    until_in_game(&g2, &mut [&mut fresh]);
    for _ in 0..80 {
        fresh.pump().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        fresh.got.iter().any(|m| m.first() == Some(&0x04)),
        "the fresh join did not load: {:02X?}",
        fresh.got
    );
    let left = character_view(&resumed.got, "Climber");
    let right = character_view(&fresh.got, "Climber");
    let want = Some(&(level as u32));
    assert_eq!(
        left.stats.get(&12),
        want,
        "the resumed view has the level-up"
    );
    assert_eq!(right.stats.get(&12), want, "the fresh view has it too");
    assert_eq!(left, right, "the resumed view equals a fresh join's");
}

/// A resume closes an open vendor trade on the game side, removes the
/// player from the NPC's list and sends the client its close.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_resume_closes_an_open_vendor_trade() {
    let s = Server::new("resume-vendor");
    let g = s.start(1, 0);
    let mut p = Player::join(&g, s.seat(1, "Shopper", Some(0)), "Shopper", 0).unwrap();
    until_in_game(&g, &mut [&mut p]);
    let client = p.client();
    let (npc_guid, player) = g
        .with_thread(move |i| {
            let sim = &mut i.hub.host.game;
            let player = sim.player_of(client).unwrap();
            let npc = sim
                .game
                .lists
                .units_of_type(d2_sim::units::UnitType::Monster)
                .into_iter()
                .find(|&u| sim.world.state.lists.contains_key(&u))
                .expect("an NPC with an interaction list");
            let guid = sim
                .events
                .action
                .sys
                .units
                .get(npc)
                .map(|r| r.guid)
                .unwrap();
            sim.world
                .state
                .lists
                .get_mut(&npc)
                .unwrap()
                .nodes
                .insert(0, (player, d2_sim::world::npc::talk::TRADING));
            sim.events
                .action
                .sys
                .units
                .get_mut(player)
                .unwrap()
                .interact
                .set(1, guid);
            (guid, player)
        })
        .unwrap();
    let old_token = p.welcome.resume;
    drop(p);
    std::thread::sleep(Duration::from_millis(100));
    let (end, server_end) = pair();
    let body = Arc::new(Mutex::new(None));
    g.resume(
        old_token,
        TestStream {
            end: server_end,
            welcome: body.clone(),
        },
    )
    .unwrap();
    let body = body.lock().unwrap().take().expect("a resume welcome");
    let welcome = d2_net::wire::parse_answer(&body).expect("a game welcome");
    let mut r = Player {
        end,
        welcome,
        joined: true,
        got: Vec::new(),
    };
    for _ in 0..40 {
        r.pump().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        r.got.iter().any(|m| m == &vec![0x77, 0x0C]),
        "the resumed client was told to close: {:02X?}",
        r.got
    );
    let state = g
        .with_thread(move |i| {
            let sim = &mut i.hub.host.game;
            let npc = sim
                .game
                .lists
                .find_unit(d2_sim::units::UnitType::Monster, npc_guid)
                .unwrap();
            (
                sim.events
                    .action
                    .sys
                    .units
                    .get(player)
                    .unwrap()
                    .interact
                    .get(),
                sim.world.state.lists.get(&npc).map(|l| l.state(player)),
            )
        })
        .unwrap();
    assert_eq!(state.0, None, "the player's interaction is closed");
    assert_eq!(
        state.1.flatten(),
        None,
        "the NPC's list no longer holds the player"
    );
}

/// A resume closes the open stash view: the interaction is reset and the
/// client is sent the stash close.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_resume_closes_an_open_stash() {
    let s = Server::new("resume-stash");
    let g = s.start(1, 0);
    let mut p = Player::join(&g, s.seat(1, "Keeper", Some(1)), "Keeper", 0).unwrap();
    until_in_game(&g, &mut [&mut p]);
    let client = p.client();
    let (_, player) = g
        .with_thread(move |i| {
            let sim = &mut i.hub.host.game;
            let player = sim.player_of(client).unwrap();
            let stash = sim
                .game
                .lists
                .units_of_type(d2_sim::units::UnitType::Object)
                .into_iter()
                .find(|&u| {
                    sim.events
                        .action
                        .sys
                        .units
                        .get(u)
                        .is_some_and(|r| r.class == d2_sim::world::stash::STASH_CLASS)
                })
                .expect("the town's stash");
            let guid = sim
                .events
                .action
                .sys
                .units
                .get(stash)
                .map(|r| r.guid)
                .unwrap();
            sim.events
                .action
                .sys
                .units
                .get_mut(player)
                .unwrap()
                .interact
                .set(2, guid);
            (guid, player)
        })
        .unwrap();
    let old_token = p.welcome.resume;
    drop(p);
    std::thread::sleep(Duration::from_millis(100));
    let (end, server_end) = pair();
    let body = Arc::new(Mutex::new(None));
    g.resume(
        old_token,
        TestStream {
            end: server_end,
            welcome: body.clone(),
        },
    )
    .unwrap();
    let body = body.lock().unwrap().take().expect("a resume welcome");
    let welcome = d2_net::wire::parse_answer(&body).expect("a game welcome");
    let mut r = Player {
        end,
        welcome,
        joined: true,
        got: Vec::new(),
    };
    for _ in 0..40 {
        r.pump().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        r.got.iter().any(|m| m == &vec![0x77, 0x11]),
        "the resumed client was told to close the stash: {:02X?}",
        r.got
    );
    let closed = g
        .with_thread(move |i| {
            let sim = &mut i.hub.host.game;
            sim.events
                .action
                .sys
                .units
                .get(player)
                .unwrap()
                .interact
                .get()
        })
        .unwrap();
    assert_eq!(closed, None, "the stash interaction is closed");
}

/// A resume closes a trade with another player on both sides: the partner
/// is told and its interaction is reset.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_resume_closes_a_player_trade_on_both_sides() {
    let s = Server::new("resume-ptrade");
    let g = s.start(1, 0);
    let mut a = Player::join(&g, s.seat(1, "Seller", Some(0)), "Seller", 0).unwrap();
    let mut b = Player::join(&g, s.seat(2, "Buyer", Some(1)), "Buyer", 0).unwrap();
    until_in_game(&g, &mut [&mut a, &mut b]);
    let (ca, cb) = (a.client(), b.client());
    let (sa, sb) = g
        .with_thread(move |i| {
            let sim = &mut i.hub.host.game;
            let (pa, pb) = (sim.player_of(ca).unwrap(), sim.player_of(cb).unwrap());
            let guid = |p| sim.events.action.sys.units.get(p).map(|r| r.guid).unwrap();
            let (ga, gb) = (guid(pa), guid(pb));
            sim.events
                .action
                .sys
                .units
                .get_mut(pa)
                .unwrap()
                .interact
                .set(0, gb);
            sim.events
                .action
                .sys
                .units
                .get_mut(pb)
                .unwrap()
                .interact
                .set(0, ga);
            (pa, pb)
        })
        .unwrap();
    let old_token = a.welcome.resume;
    drop(a);
    std::thread::sleep(Duration::from_millis(100));
    let (end, server_end) = pair();
    let body = Arc::new(Mutex::new(None));
    g.resume(
        old_token,
        TestStream {
            end: server_end,
            welcome: body.clone(),
        },
    )
    .unwrap();
    let body = body.lock().unwrap().take().expect("a resume welcome");
    let welcome = d2_net::wire::parse_answer(&body).expect("a game welcome");
    let mut r = Player {
        end,
        welcome,
        joined: true,
        got: Vec::new(),
    };
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        r.pump().unwrap();
        b.pump().unwrap();
        if r.got.iter().any(|m| m == &vec![0x77, 0x0C])
            && b.got.iter().any(|m| m == &vec![0x77, 0x0C])
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "no close: resumed {:02X?}, partner {:02X?}",
            r.got,
            b.got
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let state = g
        .with_thread(move |i| {
            let sim = &mut i.hub.host.game;
            let get = |p| sim.events.action.sys.units.get(p).unwrap().interact.get();
            (get(sa), get(sb))
        })
        .unwrap();
    assert_eq!(state, (None, None), "both sides are closed");
}

/// A panic inside a resume drops only that session: the game and the other
/// player keep running, and the dropped character is saved.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_panicking_resume_drops_only_that_session() {
    let mut s = Server::new("resume-panic");
    let g = s.start(1, 0);
    let mut a = Player::join(&g, s.seat(1, "Unlucky", Some(0)), "Unlucky", 0).unwrap();
    let mut b = Player::join(&g, s.seat(2, "Bystander", Some(1)), "Bystander", 0).unwrap();
    until_in_game(&g, &mut [&mut a, &mut b]);
    let old_token = a.welcome.resume;
    drop(a);
    for _ in 0..40 {
        b.pump().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    g.with_thread(|i| i.panic_next_resume()).unwrap();
    let (_end, server_end) = pair();
    let body = Arc::new(Mutex::new(None));
    assert_eq!(
        g.resume(
            old_token,
            TestStream {
                end: server_end,
                welcome: body.clone(),
            }
        )
        .err(),
        Some(ErrorCode::OTHER),
        "the resume failed"
    );
    let mut seen = Vec::new();
    s.wait_event(
        &mut seen,
        |e| matches!(e, GameEvent::Left { character, .. } if character == "Unlucky"),
    );
    s.wait_event(&mut seen, |e| matches!(e, GameEvent::Saved { .. }));
    assert!(!g.is_closed(), "the game keeps running");
    assert_eq!(g.players(), 1);
    b.pump().unwrap();
    assert_eq!(
        state(&g, b.client()),
        Some(client_state::IN_GAME),
        "the other player plays on"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_new_character_keeps_the_progression_set_before_its_first_game() {
    let mut s = Server::new("new-progression");
    let g = s.start(1, 0);
    let seat = s.seat_with(8, "Unlocked", Some(0), 5);
    let file = seat.file.clone();
    let mut p = Player::join(&g, seat, "Unlocked", 0).unwrap();
    until_in_game(&g, &mut [&mut p]);
    // Save and Exit: the first save of a character that was never played.
    p.send(Lane::System, vec![0x69]);
    let mut seen = Vec::new();
    let saved = s.wait_event(&mut seen, |e| matches!(e, GameEvent::Saved { .. }));
    let GameEvent::Saved { summary, .. } = saved else {
        unreachable!()
    };
    assert_eq!(d2_formats::d2s::status::progression(summary.status), 5);
    assert_eq!(summary.difficulties, 0b011, "Nightmare is open at 5");
    let stored = read_file_record(&file, "Unlocked");
    assert_eq!(
        d2_formats::d2s::header_status(&stored),
        Some(summary.status),
        "the first save kept the seat's progression"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_join_that_names_another_character_is_dropped() {
    let mut s = Server::new("mismatch");
    let g = s.start(1, 0);
    let mut honest = Player::join(&g, s.seat(1, "Honest", Some(0)), "Honest", 0).unwrap();
    until_in_game(&g, &mut [&mut honest]);
    let mut liar = Player::join(&g, s.seat(2, "Liar", Some(0)), "Honest", 0).unwrap();
    let mut seen = Vec::new();
    s.wait_event(
        &mut seen,
        |e| matches!(e, GameEvent::Left { character, .. } if character == "Liar"),
    );
    assert!(liar.pump().is_err() || state(&g, liar.client()).is_none());
    assert_eq!(g.players(), 1);
    honest.pump().unwrap();
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_game_decides_the_difficulty_not_the_join() {
    let s = Server::new("difficulty");
    let g = s.start(1, 0);
    // The client asks for Hell in a Normal game.
    let mut p = Player::join(&g, s.seat(1, "Climber", Some(3)), "Climber", 2).unwrap();
    until_in_game(&g, &mut [&mut p]);
    let flags = p
        .got
        .iter()
        .find(|m| m.first() == Some(&0x01))
        .expect("S→C 0x01");
    assert_eq!(flags[1], 0, "S→C 0x01 says Normal: {flags:02X?}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn seats_are_refused_when_the_character_is_in_or_the_game_is_full() {
    let s = Server::with(
        "full",
        GamesConfig {
            max_players: 1,
            ..config()
        },
    );
    let g = s.start(1, 0);
    let _p = Player::join(&g, s.seat(1, "Solo", Some(0)), "Solo", 0).unwrap();
    assert_eq!(
        Player::join(&g, s.seat(1, "Solo", None), "Solo", 0).err(),
        Some(ErrorCode::CHAR_IN_GAME)
    );
    assert_eq!(
        Player::join(&g, s.seat(2, "Other", Some(0)), "Other", 0).err(),
        Some(ErrorCode::GAME_FULL)
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn shutdown_saves_everyone_and_closes() {
    let mut s = Server::new("shutdown");
    let g = s.start(1, 0);
    let seat = s.seat(3, "Stayer", Some(2));
    let file = seat.file.clone();
    let mut p = Player::join(&g, seat, "Stayer", 0).unwrap();
    until_in_game(&g, &mut [&mut p]);
    s.games.shutdown_all(Duration::from_secs(20));
    let mut seen = s.drain();
    s.wait_event(&mut seen, |e| matches!(e, GameEvent::Saved { .. }));
    s.wait_event(&mut seen, |e| {
        matches!(e, GameEvent::Closed { game: 1, .. })
    });
    assert!(file.exists(), "the character was saved at shutdown");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_game_nobody_enters_closes_and_a_panicking_game_closes_alone() {
    let mut s = Server::new("close");
    let unused = s.start(1, 0);
    std::thread::sleep(Duration::from_millis(300));
    s.games.reap();
    let mut seen = Vec::new();
    s.wait_event(&mut seen, |e| {
        *e == GameEvent::Closed {
            game: 1,
            reason: ErrorCode::GAME_CLOSED,
        }
    });
    assert!(unused.is_closed());
    assert!(s.games.get(1).is_none());

    let broken = s.start(2, 0);
    let ok = s.start(3, 0);
    let mut a = Player::join(&broken, s.seat(1, "Unlucky", Some(0)), "Unlucky", 0).unwrap();
    let mut b = Player::join(&ok, s.seat(2, "Lucky", Some(0)), "Lucky", 0).unwrap();
    until_in_game(&broken, &mut [&mut a]);
    until_in_game(&ok, &mut [&mut b]);
    let _ = broken.with_thread(|_: &mut Instance| -> () { panic!("a game fault") });
    s.wait_event(&mut seen, |e| {
        matches!(e, GameEvent::Closed { game: 2, .. })
    });
    // The other game plays on.
    b.pump().unwrap();
    assert_eq!(
        state(&ok, b.client()),
        Some(client_state::IN_GAME),
        "the other game is unaffected"
    );
    assert!(!ok.is_closed());
}
