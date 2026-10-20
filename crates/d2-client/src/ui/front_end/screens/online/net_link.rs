//! The online screens' link to a real server (the `online` feature): the
//! screens' [`Request`]s become control-stream messages on the shared
//! [`OnlineSession`], and what the server sends becomes [`Event`]s.
//!
//! Connecting waits for the network, so it runs on a worker thread; its
//! answer arrives as an event. Every other request only queues its body,
//! and [`OnlineLink::poll`] reads what has arrived without waiting, so the
//! front end's 40 ms tick never blocks. The session stays in the
//! [`OnlineSession`] after the lobby's `JoinOk`: the game joins through it
//! and the player comes back to the lobby on the same session.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc;

use d2_net::msg::account::{AccountEvent, AccountRequest, CharSummary};
use d2_net::msg::lobby::{LobbyEvent, LobbyRequest};
use d2_net::msg::{kind as ranges, ErrorCode, ErrorMsg};
use d2_net::Timing;
use d2_online::{OnlineError, Session};

use super::link::{CharInfo, Event, Failure, GameInfo, Joined, OnlineLink, Request};
use super::{Online, OnlineHandle, ServerEntry};
use crate::app::online::{client_info, OnlineSession, ServerList};
use crate::ui::front_end::Registry;

/// What the screens show for a failure the session reports.
pub fn failure(e: &OnlineError) -> Failure {
    Failure {
        code: e.code().unwrap_or(ErrorCode(0)),
        wait_s: e.wait_s(),
        text: e.message(),
    }
}

fn char_info(c: &CharSummary) -> CharInfo {
    CharInfo {
        name: c.name.clone(),
        class: c.class,
        level: c.level,
        status: c.status,
        difficulties: c.difficulties,
        look: c.look.clone(),
        last_played: c.last_played,
    }
}

fn game_info(g: &d2_net::msg::lobby::GameInfo) -> GameInfo {
    GameInfo {
        id: g.game_id,
        name: g.name.clone(),
        difficulty: g.difficulty,
        players: g.players,
        max_players: g.max_players,
        locked: g.locked,
        hardcore: g.hardcore,
        expansion: g.expansion,
        joinable: g.joinable,
    }
}

/// The screens' entries for the launcher's server list.
pub fn entries(list: &ServerList) -> Vec<ServerEntry> {
    list.servers
        .iter()
        .map(|s| ServerEntry {
            id: s.id.clone(),
            name: s.name.clone(),
            address: s.address.clone(),
        })
        .collect()
}

/// The link over the shared session.
pub struct NetLink {
    session: OnlineSession,
    list: ServerList,
    /// The connect worker's answer, while one runs.
    connecting: Option<mpsc::Receiver<Event>>,
    out: VecDeque<Event>,
    /// Game names by id, from the list, for the join's loading screen.
    names: HashMap<u32, String>,
}

impl NetLink {
    pub fn new(session: OnlineSession, list: ServerList) -> Self {
        NetLink {
            session,
            list,
            connecting: None,
            out: VecDeque::new(),
            names: HashMap::new(),
        }
    }

    fn lost(&mut self, text: &str) {
        self.out.push_back(Event::Disconnected(Failure {
            code: ErrorCode(0),
            wait_s: 0,
            text: text.into(),
        }));
    }

    fn send_body(&mut self, body: Vec<u8>) {
        let r = match self.session.0.lock() {
            Ok(mut slot) => match slot.as_mut() {
                Some(s) => s.send(body).map_err(|e| failure(&e)),
                None => Err(Failure {
                    code: ErrorCode(0),
                    wait_s: 0,
                    text: "Not connected to the server.".into(),
                }),
            },
            Err(_) => Err(Failure {
                code: ErrorCode(0),
                wait_s: 0,
                text: "The online session is not available.".into(),
            }),
        };
        if let Err(f) = r {
            self.drop_session();
            self.out.push_back(Event::Disconnected(f));
        }
    }

    fn drop_session(&mut self) {
        if let Ok(mut slot) = self.session.0.lock() {
            if let Some(s) = slot.take() {
                s.close();
            }
        }
    }

    fn connect(&mut self, index: usize) {
        if self.connecting.is_some() {
            return;
        }
        let Some(spec) = self.list.servers.get(index).map(|s| s.spec()) else {
            self.lost(crate::app::online::NO_LIST);
            return;
        };
        // A new connection replaces the old one.
        self.drop_session();
        let (tx, rx) = mpsc::channel();
        let session = self.session.clone();
        std::thread::Builder::new()
            .name("online-connect".into())
            .spawn(move || {
                let ev = match Session::connect(&spec, &client_info(), Timing::default()) {
                    Ok(s) => {
                        let w = s.welcome();
                        let ev = Event::Connected {
                            server_name: w.server_name.clone(),
                            registration_open: w.registration_open,
                        };
                        if let Ok(mut slot) = session.0.lock() {
                            *slot = Some(s);
                        }
                        ev
                    }
                    Err(e) => Event::Disconnected(failure(&e)),
                };
                let _ = tx.send(ev);
            })
            .expect("a thread for connecting");
        self.connecting = Some(rx);
    }

    /// One S→C body as the screens' event (`None`: nothing they show).
    fn event(&mut self, body: &[u8]) -> Option<Event> {
        let k = *body.first()?;
        if k == d2_net::msg::kind::ERROR {
            let e = ErrorMsg::parse(body).ok()?;
            return Some(Event::Error(Failure {
                code: e.code,
                wait_s: e.wait_s,
                text: e.text,
            }));
        }
        if ranges::ACCOUNT.contains(&k) {
            return Some(match AccountEvent::parse(body).ok()? {
                AccountEvent::LoggedIn {
                    account,
                    max_characters,
                } => Event::LoggedIn {
                    account,
                    max_characters,
                },
                AccountEvent::CharList(l) => Event::Chars(l.iter().map(char_info).collect()),
                AccountEvent::CharCreated(c) => Event::CharCreated(char_info(&c)),
                AccountEvent::CharDeleted { name } => Event::CharDeleted(name),
                AccountEvent::CharSelected(c) => Event::CharSelected(char_info(&c)),
            });
        }
        if ranges::LOBBY.contains(&k) {
            return Some(match LobbyEvent::parse(body).ok()? {
                LobbyEvent::GameListBegin { .. } => {
                    self.names.clear();
                    Event::GameListBegin
                }
                LobbyEvent::GameEntry(g) => {
                    self.names.insert(g.game_id, g.name.clone());
                    Event::Game(game_info(&g))
                }
                LobbyEvent::GameListEnd => Event::GameListEnd,
                LobbyEvent::GameGone { game_id } => Event::GameGone(game_id),
                LobbyEvent::JoinOk {
                    game_id,
                    ticket,
                    difficulty,
                } => Event::JoinOk(Joined {
                    game_id,
                    ticket,
                    difficulty,
                    name: self.names.get(&game_id).cloned().unwrap_or_default(),
                }),
                LobbyEvent::ChatLine { from, text, .. } => Event::ChatLine { from, text },
            });
        }
        None
    }
}

impl OnlineLink for NetLink {
    fn send(&mut self, r: Request) {
        let body = match r {
            Request::Connect { index } => return self.connect(index),
            Request::Logout => {
                self.send_body(AccountRequest::Logout.body());
                self.drop_session();
                // The screens asked for it: no event.
                self.out.retain(|e| !matches!(e, Event::Disconnected(_)));
                return;
            }
            Request::Register { account, password } => {
                AccountRequest::Register { account, password }.body()
            }
            Request::Login { account, password } => {
                AccountRequest::Login { account, password }.body()
            }
            Request::CharList => AccountRequest::CharList.body(),
            Request::CharCreate {
                name,
                class,
                hardcore,
                expansion,
            } => AccountRequest::CharCreate {
                name,
                class,
                hardcore,
                expansion,
            }
            .body(),
            Request::CharDelete { name, password } => {
                AccountRequest::CharDelete { name, password }.body()
            }
            Request::CharSelect { name } => AccountRequest::CharSelect { name }.body(),
            Request::ListGames => LobbyRequest::ListGames.body(),
            Request::CreateGame {
                name,
                password,
                difficulty,
            } => LobbyRequest::CreateGame {
                name,
                password,
                difficulty,
                max_players: 0,
            }
            .body(),
            Request::JoinGame { game_id, password } => {
                LobbyRequest::JoinGame { game_id, password }.body()
            }
            Request::LeaveLobby => LobbyRequest::LeaveLobby.body(),
            Request::Chat { text } => LobbyRequest::Chat { text }.body(),
        };
        self.send_body(body);
    }

    fn poll(&mut self) -> Option<Event> {
        if let Some(e) = self.out.pop_front() {
            return Some(e);
        }
        if let Some(rx) = &self.connecting {
            match rx.try_recv() {
                Ok(ev) => {
                    self.connecting = None;
                    return Some(ev);
                }
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.connecting = None;
                    return Some(Event::Disconnected(Failure {
                        code: ErrorCode(0),
                        wait_s: 0,
                        text: String::new(),
                    }));
                }
            }
        }
        loop {
            let polled = match self.session.0.lock() {
                Ok(mut slot) => slot.as_mut().map(|s| s.poll()),
                Err(_) => None,
            };
            match polled {
                None | Some(Ok(None)) => return None,
                Some(Ok(Some(body))) => {
                    if let Some(ev) = self.event(&body) {
                        return Some(ev);
                    }
                }
                Some(Err(e)) => {
                    self.drop_session();
                    return Some(Event::Disconnected(failure(&e)));
                }
            }
        }
    }
}

/// Registers the online screens on the real server: the launcher's list
/// (or the reason there is none, shown on the login screen) and the shared
/// session. Returns the screens' handle for the host's hooks
/// ([`Online::take_joined`], [`Online::lost`], [`Online::select_server`]).
pub fn register(reg: &mut Registry, session: &OnlineSession) -> OnlineHandle {
    let (list, problem) = match ServerList::load() {
        Ok(l) => (l, None),
        // A damaged list says so; a missing one is the login screen's own
        // text.
        Err(e) if e != crate::app::online::NO_LIST => (ServerList::default(), Some(e)),
        Err(_) => (ServerList::default(), None),
    };
    let servers = entries(&list);
    let online = Online::new(Box::new(NetLink::new(session.clone(), list)), servers);
    online.borrow_mut().state.notice = problem;
    super::register_with(reg, online.clone());
    online
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_net::msg::ErrorCode;

    fn link() -> NetLink {
        NetLink::new(OnlineSession::default(), ServerList::default())
    }

    /// The server's bodies become the screens' events: errors keep their
    /// code, wait and words; a join names the game from the list.
    #[test]
    fn bodies_become_events() {
        let mut l = link();
        let mut e = ErrorMsg::new(0x46, ErrorCode::WRONG_PASSWORD);
        e.wait_s = 3;
        assert_eq!(
            l.event(&e.body()),
            Some(Event::Error(Failure {
                code: ErrorCode::WRONG_PASSWORD,
                wait_s: 3,
                text: ErrorCode::WRONG_PASSWORD.text().into(),
            }))
        );
        let summary = CharSummary {
            name: "Werwer".into(),
            class: 4,
            level: 30,
            status: 0x0520,
            difficulties: 0b011,
            look: vec![1; 32],
            last_played: 9,
        };
        assert_eq!(
            l.event(&AccountEvent::CharSelected(summary.clone()).body()),
            Some(Event::CharSelected(char_info(&summary)))
        );
        let g = d2_net::msg::lobby::GameInfo {
            game_id: 5,
            name: "cows".into(),
            difficulty: 1,
            players: 2,
            max_players: 8,
            locked: true,
            hardcore: false,
            expansion: true,
            joinable: 0,
        };
        assert_eq!(
            l.event(&LobbyEvent::GameEntry(g.clone()).body()),
            Some(Event::Game(game_info(&g)))
        );
        let join = LobbyEvent::JoinOk {
            game_id: 5,
            ticket: [4; 16],
            difficulty: 1,
        };
        assert_eq!(
            l.event(&join.body()),
            Some(Event::JoinOk(Joined {
                game_id: 5,
                ticket: [4; 16],
                difficulty: 1,
                name: "cows".into(),
            }))
        );
        assert_eq!(
            l.event(
                &LobbyEvent::ChatLine {
                    from: "A".into(),
                    text: "hi".into(),
                    at: 1
                }
                .body()
            ),
            Some(Event::ChatLine {
                from: "A".into(),
                text: "hi".into()
            })
        );
        // Bodies of other areas and broken ones show nothing.
        assert_eq!(l.event(&[0x60, 1]), None);
        assert_eq!(l.event(&[0x42]), None);
    }

    /// Without a connection a request fails at once with a reason, and a
    /// server the list doesn't have says where the list comes from.
    #[test]
    fn requests_without_a_session_fail_with_a_reason() {
        let mut l = link();
        l.send(Request::CharList);
        assert!(
            matches!(l.poll(), Some(Event::Disconnected(f)) if f.text.contains("Not connected"))
        );
        l.send(Request::Connect { index: 0 });
        assert!(matches!(
            l.poll(),
            Some(Event::Disconnected(f)) if f.text == crate::app::online::NO_LIST
        ));
        assert_eq!(l.poll(), None);
    }

    #[test]
    fn session_failures_keep_code_wait_and_words() {
        let f = failure(&OnlineError::Server {
            request_kind: 0x21,
            code: ErrorCode::RATE_LIMITED,
            wait_s: 30,
            text: "slow down".into(),
        });
        assert_eq!((f.code, f.wait_s), (ErrorCode::RATE_LIMITED, 30));
        assert!(f.text.contains("30 seconds"));
        assert_eq!(failure(&OnlineError::TimedOut).code, ErrorCode(0));
    }
}
