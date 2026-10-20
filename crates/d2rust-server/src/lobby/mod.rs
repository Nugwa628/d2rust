//! The lobby: the list of open games, creating and joining them with the
//! password and difficulty rules, the join tickets, and the lobby chat
//! (multiplayer contracts sections 3.6, 5 and 6).
//!
//! - [`state`]: every rule, with no I/O and a clock the caller passes.
//! - [`Lobby`]: the rules on the server: it answers the lobby's control
//!   messages ([`LobbyApi::handle`]), starts games on `CreateGame`, hands a
//!   join's seat to the game stream's `Attach` ([`LobbyApi::redeem`]), and
//!   keeps every listening session's game list current from the games'
//!   events. The selected character is the session's stored summary, never
//!   the client's word.

pub mod state;

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use d2_net::msg::account::CharSummary;
use d2_net::msg::lobby::{GameInfo, LobbyEvent, LobbyRequest};
use d2_net::msg::ErrorCode;

use crate::accounts::{unix_now, Accounts, LimitMode};
use crate::config::ServerConfig;
use crate::denied::Denied;
use crate::games::{GameEvent, GameHandle, GameSpec, Games, Seat};
use crate::session::{LobbyApi, SessionCtx, Sessions};

pub use state::{Limits, LobbyChar, LobbyState, NewGame, OpenGame, Refusal, Ticket, TICKET_TTL};

/// The lobby's view of a session's selected character.
pub fn lobby_char(c: &CharSummary) -> LobbyChar {
    LobbyChar {
        name: c.name.clone(),
        status: c.status,
        difficulties: c.difficulties,
    }
}

/// The lobby's limits from the server config: `max_games` and the
/// players per game.
pub fn limits(cfg: &ServerConfig) -> Limits {
    let players = u8::try_from(cfg.max_players).unwrap_or(u8::MAX);
    Limits {
        max_games: cfg.max_games,
        default_players: players,
        players_cap: players,
        test_windows: cfg.accounts.limits == LimitMode::Test,
    }
}

impl From<Refusal> for Denied {
    fn from(r: Refusal) -> Self {
        Denied::wait(r.code, r.wait_s)
    }
}

/// The lobby on the server.
pub struct Lobby {
    state: Mutex<LobbyState>,
    games: Games,
    sessions: Sessions,
    accounts: Accounts,
    /// The open games' handles, by id.
    handles: Mutex<HashMap<u32, GameHandle>>,
    /// What each issued ticket seats, for the game stream's `Attach`.
    seats: Mutex<HashMap<[u8; 16], (GameHandle, Seat)>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn ticket() -> [u8; 16] {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the operating system has a random source");
    b
}

impl Lobby {
    pub fn new(limits: Limits, games: Games, sessions: Sessions, accounts: Accounts) -> Self {
        Lobby {
            state: Mutex::new(LobbyState::new(limits)),
            games,
            sessions,
            accounts,
            handles: Mutex::new(HashMap::new()),
            seats: Mutex::new(HashMap::new()),
        }
    }

    /// The open games (for the status file and tests).
    pub fn games(&self) -> Vec<OpenGame> {
        lock(&self.state).games().cloned().collect()
    }

    /// Every listener's new view of game `id`: its entry, or `GameGone`.
    fn broadcast(&self, id: u32) {
        let views = lock(&self.state).views_of(id);
        for (session, view) in views {
            let ev = match view {
                Some(g) => LobbyEvent::GameEntry(g),
                None => LobbyEvent::GameGone { game_id: id },
            };
            self.sessions.send(session, ev.body());
        }
    }

    /// Expired tickets free their seats; the lists show it.
    fn expire(&self, now: Instant) {
        let gone = lock(&self.state).expire(now);
        let mut games: Vec<u32> = Vec::new();
        {
            let mut seats = lock(&self.seats);
            for (t, g) in gone {
                seats.remove(&t);
                games.push(g);
            }
        }
        games.sort_unstable();
        games.dedup();
        for g in games {
            self.broadcast(g);
        }
    }

    /// Hold a seat in `handle`'s game for the session's character and send
    /// `JoinOk` with its ticket.
    async fn seat(
        &self,
        s: &SessionCtx,
        c: &LobbyChar,
        handle: GameHandle,
        now: Instant,
    ) -> Result<(), Denied> {
        let (account, _) = s.account.clone().ok_or(ErrorCode::NOT_LOGGED_IN)?;
        let cs = self.accounts.seat(account, &c.name).await?;
        let seat = Seat {
            account,
            character: cs.summary.name.clone(),
            file: cs.file,
            new: cs.new,
        };
        let t = ticket();
        let id = handle.id();
        let difficulty = handle.spec().difficulty;
        lock(&self.state).issue(s.id, c, id, t, now)?;
        lock(&self.seats).insert(t, (handle, seat));
        s.out.send(
            LobbyEvent::JoinOk {
                game_id: id,
                ticket: t,
                difficulty,
            }
            .body(),
        );
        self.broadcast(id);
        Ok(())
    }

    /// `CreateGame`: the rules, then the game (listed at once), then the
    /// creator's seat.
    async fn create(
        &self,
        s: &SessionCtx,
        c: &LobbyChar,
        new: NewGame<'_>,
        now: Instant,
    ) -> Result<(), Denied> {
        let handle = {
            let mut st = lock(&self.state);
            match st.check_create(s.id, c, new, now) {
                Err(r) => return Err(r.into()),
                Ok(g) => {
                    let spec = GameSpec {
                        id: g.id,
                        name: g.name.clone(),
                        difficulty: g.difficulty,
                        hardcore: g.hardcore,
                        expansion: g.expansion,
                        max_players: g.max_players,
                    };
                    let events = self.games.events().ok_or(ErrorCode::OTHER)?;
                    let handle = self.games.start(spec, events)?;
                    st.opened(g);
                    handle
                }
            }
        };
        lock(&self.handles).insert(handle.id(), handle.clone());
        self.broadcast(handle.id());
        self.seat(s, c, handle, now).await
    }

    /// `JoinGame`: the rules, then the seat.
    async fn join(
        &self,
        s: &SessionCtx,
        c: &LobbyChar,
        game_id: u32,
        password: &str,
        now: Instant,
    ) -> Result<(), Denied> {
        let checked = lock(&self.state).check_join(s.id, c, game_id, password, now);
        checked?;
        let handle = lock(&self.handles)
            .get(&game_id)
            .filter(|h| !h.is_closed())
            .cloned()
            .ok_or(ErrorCode::GAME_NOT_FOUND)?;
        self.seat(s, c, handle, now).await
    }

    fn list(&self, s: &SessionCtx, c: &LobbyChar) {
        let list: Vec<GameInfo> = lock(&self.state).subscribe(s.id, c);
        let count = u16::try_from(list.len()).unwrap_or(u16::MAX);
        s.out.send(LobbyEvent::GameListBegin { count }.body());
        for g in list {
            s.out.send(LobbyEvent::GameEntry(g).body());
        }
        s.out.send(LobbyEvent::GameListEnd.body());
    }

    fn chat(
        &self,
        s: &SessionCtx,
        c: &LobbyChar,
        text: String,
        now: Instant,
    ) -> Result<(), Denied> {
        let listeners = {
            let mut st = lock(&self.state);
            st.check_chat(s.id, now)?;
            st.listeners()
        };
        let body = LobbyEvent::ChatLine {
            from: c.name.clone(),
            text,
            at: u64::try_from(unix_now()).unwrap_or(0),
        }
        .body();
        for id in listeners {
            self.sessions.send(id, body.clone());
        }
        Ok(())
    }
}

impl LobbyApi for Lobby {
    async fn handle(&self, s: &mut SessionCtx, body: &[u8]) -> Result<(), Denied> {
        let req = LobbyRequest::parse(body).map_err(|_| ErrorCode::MALFORMED)?;
        let c = lobby_char(s.character.as_ref().ok_or(ErrorCode::NO_CHAR_SELECTED)?);
        let now = Instant::now();
        self.expire(now);
        match req {
            LobbyRequest::ListGames => self.list(s, &c),
            LobbyRequest::LeaveLobby => lock(&self.state).unsubscribe(s.id),
            LobbyRequest::Chat { text } => self.chat(s, &c, text, now)?,
            LobbyRequest::CreateGame {
                name,
                password,
                difficulty,
                max_players,
            } => {
                let new = NewGame {
                    name: &name,
                    password: &password,
                    difficulty,
                    max_players,
                };
                self.create(s, &c, new, now).await?;
            }
            LobbyRequest::JoinGame { game_id, password } => {
                self.join(s, &c, game_id, &password, now).await?;
            }
        }
        Ok(())
    }

    fn redeem(&self, s: &SessionCtx, ticket: [u8; 16]) -> Result<(GameHandle, Seat), Denied> {
        let now = Instant::now();
        // The seat goes only with a ticket the state takes: another
        // session's try must not void it. (One lock at a time: the state's
        // guard ends with this statement.)
        let (taken, still_issued, freed) = {
            let mut st = lock(&self.state);
            let r = st.redeem(s.id, ticket, now);
            let issued = st.ticket(&ticket).is_some();
            let freed = r.as_ref().err().and_then(|e| e.freed);
            (r, issued, freed)
        };
        if let Some(game) = freed {
            // An expired ticket freed its seat: the list shows it again.
            self.broadcast(game);
        }
        match taken {
            Ok(_) => lock(&self.seats)
                .remove(&ticket)
                .ok_or(ErrorCode::TICKET_INVALID.into()),
            Err(e) => {
                if !still_issued {
                    // Expired or unknown: nothing holds the seat any more.
                    lock(&self.seats).remove(&ticket);
                }
                Err(e.refusal.into())
            }
        }
    }

    fn on_game_event(&self, ev: GameEvent) {
        match ev {
            GameEvent::Players { game, players } => {
                lock(&self.state).players(game, players);
                self.broadcast(game);
            }
            GameEvent::Left {
                game, character, ..
            } => {
                lock(&self.state).left(&character);
                self.broadcast(game);
            }
            GameEvent::Closed { game, .. } => {
                lock(&self.state).closed(game);
                lock(&self.handles).remove(&game);
                lock(&self.seats).retain(|_, (h, _)| h.id() != game);
                self.broadcast(game);
            }
            GameEvent::Saved { .. } => {}
        }
    }

    fn session_ended(&self, s: &SessionCtx) {
        lock(&self.state).session_ended(s.id);
        let held: Vec<u32> = {
            let mut seats = lock(&self.seats);
            let mut games = Vec::new();
            seats.retain(|_, (h, seat)| {
                let mine = s.account.as_ref().is_some_and(|(a, _)| *a == seat.account)
                    && s.character
                        .as_ref()
                        .is_some_and(|c| c.name.eq_ignore_ascii_case(&seat.character));
                if mine {
                    games.push(h.id());
                }
                !mine
            });
            games
        };
        for g in held {
            self.broadcast(g);
        }
    }

    fn character_in_game(&self, name_key: &str) -> bool {
        lock(&self.state).character_in_game(name_key)
    }

    fn caps(&self) -> &'static [&'static str] {
        &["lobby", "chat"]
    }
}
