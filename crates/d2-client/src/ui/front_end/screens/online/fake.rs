//! An in-memory server behind [`OnlineLink`] for the online screens' tests
//! and offscreen captures. It answers with the contract's rules (accounts,
//! characters, the lobby's password, difficulty and kind rules), so a
//! screen test drives the same answers the real server gives. It is not the
//! server: no network, no persistence, no rate limits.

use std::collections::VecDeque;

use d2_formats::d2s::status;

use super::link::{CharInfo, ErrorCode, Event, Failure, GameInfo, Joined, OnlineLink, Request};

#[derive(Clone, Debug)]
struct Account {
    name: String,
    password: String,
    chars: Vec<CharInfo>,
}

#[derive(Clone, Debug)]
struct Game {
    info: GameInfo,
    password: String,
}

/// The fake server's state; every field is open so a test can set up a
/// scene (games already running, a character with Hell unlocked).
#[derive(Clone, Debug)]
pub struct FakeServer {
    pub server_name: String,
    pub registration_open: bool,
    /// Every request is answered with this failure (0: none); `Connect`
    /// fails with it as `Disconnected`.
    pub refuse_connect: Option<Failure>,
    /// Chat answers `NotSupported`.
    pub chat_off: bool,
    /// Requests are answered only when [`FakeServer::release`] is called
    /// (captures of the "please wait" box).
    pub hold: bool,
    pub max_characters: u8,
    accounts: Vec<Account>,
    games: Vec<Game>,
    next_game: u32,
    connected: bool,
    me: Option<usize>,
    selected: Option<CharInfo>,
    in_lobby: bool,
    held: VecDeque<Request>,
    out: VecDeque<Event>,
    /// Every request seen, in order (tests read it).
    pub seen: Vec<Request>,
}

impl Default for FakeServer {
    fn default() -> Self {
        FakeServer {
            server_name: "D2RUST Test".into(),
            registration_open: true,
            refuse_connect: None,
            chat_off: false,
            hold: false,
            max_characters: 16,
            accounts: Vec::new(),
            games: Vec::new(),
            next_game: 1,
            connected: false,
            me: None,
            selected: None,
            in_lobby: false,
            held: VecDeque::new(),
            out: VecDeque::new(),
            seen: Vec::new(),
        }
    }
}

/// A character for a scene: `progression` sets the unlocked difficulties.
pub fn character(name: &str, class: u8, level: u8, progression: u8, flags: u16) -> CharInfo {
    let st = (u16::from(progression) << 8) | flags;
    CharInfo {
        name: name.into(),
        class,
        level,
        status: st,
        difficulties: status::difficulties(st),
        look: vec![0xFF; 32],
        last_played: 0,
    }
}

fn fail(code: ErrorCode, text: &str) -> Event {
    Event::Error(Failure {
        code,
        wait_s: 0,
        text: text.into(),
    })
}

/// Account names: 2-15 ASCII letters, digits, `-`, `_`, starting with a
/// letter (contract section 4).
pub fn account_name_ok(n: &str) -> bool {
    (2..=15).contains(&n.len())
        && n.starts_with(|c: char| c.is_ascii_alphabetic())
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Game names: 1-15 letters, digits, space, `-`, `_` (contract section 6).
pub fn game_name_ok(n: &str) -> bool {
    (1..=15).contains(&n.chars().count())
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_')
        && !n.trim().is_empty()
}

impl FakeServer {
    /// An account that exists before the test logs in.
    pub fn add_account(&mut self, name: &str, password: &str, chars: Vec<CharInfo>) {
        self.accounts.push(Account {
            name: name.into(),
            password: password.into(),
            chars,
        });
    }

    /// A game somebody else opened.
    pub fn add_game(&mut self, info: GameInfo, password: &str) -> u32 {
        let id = self.next_game;
        self.next_game += 1;
        self.games.push(Game {
            info: GameInfo { id, ..info },
            password: password.into(),
        });
        id
    }

    /// A game closes (the lobby gets `GameGone`).
    pub fn close_game(&mut self, id: u32) {
        self.games.retain(|g| g.info.id != id);
        if self.in_lobby {
            self.out.push_back(Event::GameGone(id));
        }
    }

    /// Somebody chats in the lobby.
    pub fn chat_from(&mut self, from: &str, text: &str) {
        if self.in_lobby {
            self.out.push_back(Event::ChatLine {
                from: from.into(),
                text: text.into(),
            });
        }
    }

    /// The connection drops.
    pub fn drop_connection(&mut self, text: &str) {
        self.connected = false;
        self.me = None;
        self.in_lobby = false;
        self.out.push_back(Event::Disconnected(Failure {
            code: ErrorCode(0),
            wait_s: 0,
            text: text.into(),
        }));
    }

    /// Answer the held requests.
    pub fn release(&mut self) {
        self.hold = false;
        while let Some(r) = self.held.pop_front() {
            self.answer(r);
        }
    }

    /// What the selected character sees for a game.
    fn view(&self, g: &Game) -> GameInfo {
        let mut info = g.info.clone();
        info.joinable = self.selected.as_ref().map_or(0, |c| join_rule(c, &info).0);
        info
    }

    fn answer(&mut self, r: Request) {
        if let Some(f) = &self.refuse_connect {
            self.out.push_back(Event::Disconnected(f.clone()));
            return;
        }
        let me = self.me;
        match r {
            Request::Connect { .. } => {
                self.connected = true;
                self.out.push_back(Event::Connected {
                    server_name: self.server_name.clone(),
                    registration_open: self.registration_open,
                });
            }
            Request::Register { account, password } => {
                if !self.registration_open {
                    self.out
                        .push_back(fail(ErrorCode::REGISTRATION_CLOSED, "registration closed"));
                } else if !account_name_ok(&account) {
                    self.out.push_back(fail(ErrorCode::BAD_ACCOUNT_NAME, ""));
                } else if !(8..=64).contains(&password.len()) {
                    self.out
                        .push_back(fail(ErrorCode::BAD_PASSWORD, "bad password"));
                } else if self
                    .accounts
                    .iter()
                    .any(|a| a.name.eq_ignore_ascii_case(&account))
                {
                    self.out
                        .push_back(fail(ErrorCode::ACCOUNT_NAME_TAKEN, "taken"));
                } else {
                    self.add_account(&account, &password, Vec::new());
                    self.me = Some(self.accounts.len() - 1);
                    self.out.push_back(Event::LoggedIn {
                        account,
                        max_characters: self.max_characters,
                    });
                }
            }
            Request::Login { account, password } => {
                match self
                    .accounts
                    .iter()
                    .position(|a| a.name.eq_ignore_ascii_case(&account) && a.password == password)
                {
                    Some(i) => {
                        self.me = Some(i);
                        self.out.push_back(Event::LoggedIn {
                            account: self.accounts[i].name.clone(),
                            max_characters: self.max_characters,
                        });
                    }
                    None => self
                        .out
                        .push_back(fail(ErrorCode::WRONG_LOGIN, "wrong login")),
                }
            }
            Request::Logout => {
                self.connected = false;
                self.me = None;
                self.selected = None;
                self.in_lobby = false;
            }
            _ if me.is_none() => self.out.push_back(fail(ErrorCode::NOT_LOGGED_IN, "")),
            Request::CharList => {
                let a = &self.accounts[me.unwrap_or(0)];
                self.out.push_back(Event::Chars(a.chars.clone()));
            }
            Request::CharCreate {
                name,
                class,
                hardcore,
                expansion,
            } => {
                let taken = self
                    .accounts
                    .iter()
                    .flat_map(|a| &a.chars)
                    .any(|c| c.name.eq_ignore_ascii_case(&name));
                let a = &mut self.accounts[me.unwrap_or(0)];
                if taken {
                    self.out.push_back(fail(ErrorCode::CHAR_NAME_TAKEN, ""));
                } else if a.chars.len() >= usize::from(self.max_characters) {
                    self.out.push_back(fail(ErrorCode::CHAR_LIMIT, ""));
                } else {
                    let mut flags = 0;
                    if hardcore {
                        flags |= status::HARDCORE;
                    }
                    if expansion {
                        flags |= status::EXPANSION;
                    }
                    let c = character(&name, class, 1, 0, flags);
                    a.chars.insert(0, c.clone());
                    self.out.push_back(Event::CharCreated(c));
                }
            }
            Request::CharDelete { name, password } => {
                let a = &mut self.accounts[me.unwrap_or(0)];
                if a.password != password {
                    self.out.push_back(fail(ErrorCode::WRONG_LOGIN, ""));
                } else if let Some(i) = a
                    .chars
                    .iter()
                    .position(|c| c.name.eq_ignore_ascii_case(&name))
                {
                    let c = a.chars.remove(i);
                    self.out.push_back(Event::CharDeleted(c.name));
                } else {
                    self.out.push_back(fail(ErrorCode::UNKNOWN_CHAR, ""));
                }
            }
            Request::CharSelect { name } => {
                let a = &self.accounts[me.unwrap_or(0)];
                match a.chars.iter().find(|c| c.name.eq_ignore_ascii_case(&name)) {
                    Some(c) => {
                        self.selected = Some(c.clone());
                        self.out.push_back(Event::CharSelected(c.clone()));
                    }
                    None => self.out.push_back(fail(ErrorCode::UNKNOWN_CHAR, "")),
                }
            }
            _ if self.selected.is_none() => {
                self.out.push_back(fail(ErrorCode::NO_CHAR_SELECTED, ""))
            }
            Request::ListGames => {
                self.in_lobby = true;
                self.out.push_back(Event::GameListBegin);
                let views: Vec<GameInfo> = self
                    .games
                    .iter()
                    .filter(|g| self.same_kind(&g.info))
                    .map(|g| self.view(g))
                    .collect();
                for v in views {
                    self.out.push_back(Event::Game(v));
                }
                self.out.push_back(Event::GameListEnd);
            }
            Request::LeaveLobby => self.in_lobby = false,
            Request::CreateGame {
                name,
                password,
                difficulty,
            } => {
                let c = self
                    .selected
                    .clone()
                    .unwrap_or_else(|| character("", 0, 1, 0, 0));
                if c.status & (status::HARDCORE | status::DEAD) == (status::HARDCORE | status::DEAD)
                {
                    self.out.push_back(fail(ErrorCode::DEAD_HARDCORE, ""));
                } else if !game_name_ok(&name) {
                    self.out.push_back(fail(ErrorCode::BAD_GAME_NAME, ""));
                } else if password.len() > 15 {
                    self.out.push_back(fail(
                        ErrorCode::OTHER,
                        "Game passwords are at most 15 characters.",
                    ));
                } else if !status::difficulty_open(c.difficulties, difficulty) {
                    self.out.push_back(fail(ErrorCode::DIFFICULTY_LOCKED, ""));
                } else if self
                    .games
                    .iter()
                    .any(|g| g.info.name.eq_ignore_ascii_case(&name))
                {
                    self.out.push_back(fail(ErrorCode::GAME_NAME_TAKEN, ""));
                } else {
                    let info = GameInfo {
                        id: 0,
                        name: name.clone(),
                        difficulty,
                        players: 1,
                        max_players: 8,
                        locked: !password.is_empty(),
                        hardcore: c.status & status::HARDCORE != 0,
                        expansion: c.status & status::EXPANSION != 0,
                        joinable: 0,
                    };
                    let id = self.add_game(info, &password);
                    self.in_lobby = false;
                    self.out.push_back(Event::JoinOk(Joined {
                        game_id: id,
                        ticket: [7; 16],
                        difficulty,
                        name,
                    }));
                }
            }
            Request::JoinGame { game_id, password } => {
                let c = self
                    .selected
                    .clone()
                    .unwrap_or_else(|| character("", 0, 1, 0, 0));
                match self.games.iter().find(|g| g.info.id == game_id) {
                    None => self.out.push_back(fail(ErrorCode::GAME_NOT_FOUND, "")),
                    Some(g) => {
                        let rule = join_rule(&c, &g.info);
                        if rule != ErrorCode(0) {
                            self.out.push_back(fail(rule, ""));
                        } else if g.info.locked && password.is_empty() {
                            self.out.push_back(fail(ErrorCode::PASSWORD_NEEDED, ""));
                        } else if g.info.locked && g.password != password {
                            self.out.push_back(fail(ErrorCode::WRONG_PASSWORD, ""));
                        } else {
                            let j = Joined {
                                game_id,
                                ticket: [9; 16],
                                difficulty: g.info.difficulty,
                                name: g.info.name.clone(),
                            };
                            self.in_lobby = false;
                            self.out.push_back(Event::JoinOk(j));
                        }
                    }
                }
            }
            Request::Chat { text } => {
                if self.chat_off {
                    self.out
                        .push_back(fail(ErrorCode::NOT_SUPPORTED, "chat is not built"));
                } else {
                    let from = self
                        .selected
                        .as_ref()
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    self.out.push_back(Event::ChatLine { from, text });
                }
            }
        }
    }

    fn same_kind(&self, g: &GameInfo) -> bool {
        self.selected.as_ref().is_some_and(|c| {
            (c.status & status::HARDCORE != 0) == g.hardcore
                && (c.status & status::EXPANSION != 0) == g.expansion
        })
    }
}

/// Why character `c` can't join game `g` (0: it can): the contract's
/// order: dead hardcore, kind, difficulty, full.
pub fn join_rule(c: &CharInfo, g: &GameInfo) -> ErrorCode {
    let hc = c.status & status::HARDCORE != 0;
    let exp = c.status & status::EXPANSION != 0;
    if c.status & (status::HARDCORE | status::DEAD) == (status::HARDCORE | status::DEAD) {
        ErrorCode::DEAD_HARDCORE
    } else if hc != g.hardcore || exp != g.expansion {
        ErrorCode::WRONG_GAME_KIND
    } else if !status::difficulty_open(c.difficulties, g.difficulty) {
        ErrorCode::DIFFICULTY_LOCKED
    } else if g.players >= g.max_players {
        ErrorCode::GAME_FULL
    } else {
        ErrorCode(0)
    }
}

impl OnlineLink for FakeServer {
    fn send(&mut self, r: Request) {
        self.seen.push(r.clone());
        if self.hold {
            self.held.push_back(r);
        } else {
            self.answer(r);
        }
    }

    fn poll(&mut self) -> Option<Event> {
        self.out.pop_front()
    }
}

/// A [`FakeServer`] shared between a test and the screens' link.
#[derive(Clone, Default)]
pub struct SharedFake(pub std::rc::Rc<std::cell::RefCell<FakeServer>>);

impl OnlineLink for SharedFake {
    fn send(&mut self, r: Request) {
        self.0.borrow_mut().send(r);
    }

    fn poll(&mut self) -> Option<Event> {
        self.0.borrow_mut().poll()
    }
}
