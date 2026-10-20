//! The lobby's rules and state, with no I/O: the open games, who is
//! listening to the list, the join tickets, which characters are in a game,
//! and the per-session rate limits. [`super::Lobby`] wraps it in a lock and
//! does the sending; every rule of contracts sections 3.6, 5 and 6 is
//! decided here, so the tests drive it directly with a clock they set.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use d2_formats::d2s::status;
use d2_net::msg::lobby::{GameInfo, DIFFICULTIES, MAX_GAME_NAME, MAX_GAME_PASSWORD};
use d2_net::msg::ErrorCode;

/// A join ticket is valid this long (contracts 3.7).
pub const TICKET_TTL: Duration = Duration::from_secs(30);

/// What the lobby reads of the selected character: the stored summary's
/// (`d2_net::msg::account::CharSummary`) name, status word and unlocked
/// mask, never the client's word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LobbyChar {
    pub name: String,
    pub status: u16,
    /// Bit 0 Normal, bit 1 Nightmare, bit 2 Hell
    /// (`d2_formats::d2s::status::difficulties`).
    pub difficulties: u8,
}

/// The lobby's limits (from the server config).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Open games at most (`max_games`).
    pub max_games: usize,
    /// Players of a game when the creator asks for 0 (`max_players`).
    pub default_players: u8,
    /// The most players a creator may ask for.
    pub players_cap: u8,
    /// `limits = test` (contracts 10.1): every rate-limit window is divided
    /// by 60; the counts stay.
    pub test_windows: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_games: 16,
            default_players: 8,
            players_cap: 8,
            test_windows: false,
        }
    }
}

/// A sliding-window limit: at most `n` events per `window` (contracts
/// section 5).
#[derive(Clone, Debug)]
pub struct Window {
    n: usize,
    window: Duration,
    at: VecDeque<Instant>,
}

impl Window {
    pub fn new(n: usize, window: Duration) -> Self {
        Window {
            n,
            window,
            at: VecDeque::new(),
        }
    }

    /// Whether one more event fits now; `Err(wait_s)` if not. A refused try
    /// is not counted.
    pub fn check(&mut self, now: Instant) -> Result<(), u32> {
        while self
            .at
            .front()
            .is_some_and(|&t| now.duration_since(t) >= self.window)
        {
            self.at.pop_front();
        }
        if self.at.len() >= self.n {
            let oldest = self.at[0];
            let left = self.window.saturating_sub(now.duration_since(oldest));
            return Err(left.as_secs_f64().ceil().max(1.0) as u32);
        }
        Ok(())
    }

    /// Count an event (after [`Window::check`] said it fits).
    pub fn hit(&mut self, now: Instant) {
        self.at.push_back(now);
    }
}

/// One session's limits: `CreateGame` 5/min, wrong join passwords 5/min,
/// chat 5 lines / 10 s.
#[derive(Clone, Debug)]
pub struct SessionLimits {
    pub create: Window,
    pub wrong_password: Window,
    pub chat: Window,
}

impl SessionLimits {
    /// The limits, with every window divided by `div` (60 for
    /// `limits = test`, else 1).
    pub fn new(div: u32) -> Self {
        let w = |s: u64| Duration::from_secs(s) / div.max(1);
        SessionLimits {
            create: Window::new(5, w(60)),
            wrong_password: Window::new(5, w(60)),
            chat: Window::new(5, w(10)),
        }
    }
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self::new(1)
    }
}

/// A refusal: the code, and the wait for rate limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: ErrorCode,
    pub wait_s: u32,
}

impl From<ErrorCode> for Refusal {
    fn from(code: ErrorCode) -> Self {
        Refusal { code, wait_s: 0 }
    }
}

/// Why a ticket did not seat its player: the refusal, and the game whose
/// seat an expired ticket released (the caller lists that game again).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RedeemError {
    pub refusal: Refusal,
    pub freed: Option<u32>,
}

impl From<Refusal> for RedeemError {
    fn from(refusal: Refusal) -> Self {
        RedeemError {
            refusal,
            freed: None,
        }
    }
}

fn limited(wait_s: u32) -> Refusal {
    Refusal {
        code: ErrorCode::RATE_LIMITED,
        wait_s,
    }
}

/// An open game as the lobby keeps it. The password lives only here, in
/// memory, as long as the game; `Debug` shows whether there is one, never
/// the password.
#[derive(Clone, PartialEq, Eq)]
pub struct OpenGame {
    pub id: u32,
    pub name: String,
    password: String,
    pub difficulty: u8,
    pub hardcore: bool,
    pub expansion: bool,
    pub max_players: u8,
    /// Players in the game (the game's own count).
    pub players: u8,
}

impl OpenGame {
    pub fn locked(&self) -> bool {
        !self.password.is_empty()
    }
}

impl std::fmt::Debug for OpenGame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenGame")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("locked", &self.locked())
            .field("difficulty", &self.difficulty)
            .field("hardcore", &self.hardcore)
            .field("expansion", &self.expansion)
            .field("max_players", &self.max_players)
            .field("players", &self.players)
            .finish()
    }
}

/// What a secret field shows in `Debug`: nothing for an empty one, a
/// marker for a set one.
fn secret(s: &str) -> &'static str {
    if s.is_empty() {
        ""
    } else {
        "<redacted>"
    }
}

/// A join ticket: the seat it holds and who may redeem it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub session: u64,
    pub game: u32,
    /// The character's name (as stored).
    pub character: String,
    pub expires: Instant,
}

/// What a listener of the list is: its character decides which games it
/// sees and whether it can join them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Listener {
    character: LobbyChar,
}

/// The names the lobby compares: lowercase ASCII (contracts 4: unique
/// without regard to case).
pub fn name_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Game names: 1-15 letters, digits, space, `-`, `_`; not only spaces.
pub fn game_name_ok(name: &str) -> bool {
    (1..=MAX_GAME_NAME).contains(&name.chars().count())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        && !name.trim().is_empty()
}

/// Compares two passwords in time that depends only on their lengths'
/// maximum (the lobby's passwords are at most 15 bytes).
pub fn password_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let n = a.len().max(b.len());
    let mut diff = (a.len() ^ b.len()) as u8 | u8::from(a.len() != b.len());
    for i in 0..n {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

fn dead_hardcore(c: &LobbyChar) -> bool {
    c.status & (status::HARDCORE | status::DEAD) == (status::HARDCORE | status::DEAD)
}

fn kind_of(c: &LobbyChar) -> (bool, bool) {
    (
        c.status & status::HARDCORE != 0,
        c.status & status::EXPANSION != 0,
    )
}

/// A `CreateGame` request's fields. `Debug` never shows the password.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NewGame<'a> {
    pub name: &'a str,
    /// Empty: a public game.
    pub password: &'a str,
    pub difficulty: u8,
    /// 0: the server's default.
    pub max_players: u8,
}

impl std::fmt::Debug for NewGame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewGame")
            .field("name", &self.name)
            .field("password", &secret(self.password))
            .field("difficulty", &self.difficulty)
            .field("max_players", &self.max_players)
            .finish()
    }
}

/// The lobby's state.
#[derive(Default)]
pub struct LobbyState {
    pub limits: Limits,
    games: BTreeMap<u32, OpenGame>,
    next_id: u32,
    listeners: BTreeMap<u64, Listener>,
    tickets: HashMap<[u8; 16], Ticket>,
    /// Character name key -> the game it is in (or holds a ticket for)
    /// and the session that took it there.
    playing: HashMap<String, (u32, u64)>,
    /// Session -> the game it is in.
    sessions_in_game: HashMap<u64, u32>,
    /// Game -> seats a ticket seated but the game's `Players` event has
    /// not counted yet. Kept until that event so a redeem cannot overbook.
    seating: HashMap<u32, u8>,
    session_limits: HashMap<u64, SessionLimits>,
}

impl std::fmt::Debug for LobbyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LobbyState")
            .field("limits", &self.limits)
            .field("games", &self.games)
            .field("next_id", &self.next_id)
            .field("listeners", &self.listeners)
            .field("tickets", &self.tickets.len())
            .field("playing", &self.playing)
            .field("sessions_in_game", &self.sessions_in_game)
            .field("seating", &self.seating)
            .field("session_limits", &self.session_limits)
            .finish()
    }
}

impl LobbyState {
    pub fn new(limits: Limits) -> Self {
        LobbyState {
            limits,
            next_id: 1,
            ..LobbyState::default()
        }
    }

    pub fn game(&self, id: u32) -> Option<&OpenGame> {
        self.games.get(&id)
    }

    pub fn games(&self) -> impl Iterator<Item = &OpenGame> {
        self.games.values()
    }

    /// Why `c` can't join `g` (0: it can), in the contract's order: dead
    /// hardcore, kind, difficulty, full. The mask is the stored summary's.
    pub fn joinable(&self, c: &LobbyChar, g: &OpenGame) -> u16 {
        let (hc, exp) = kind_of(c);
        let code = if dead_hardcore(c) {
            ErrorCode::DEAD_HARDCORE
        } else if (hc, exp) != (g.hardcore, g.expansion) {
            ErrorCode::WRONG_GAME_KIND
        } else if !status::difficulty_open(c.difficulties, g.difficulty) {
            ErrorCode::DIFFICULTY_LOCKED
        } else if self.seats_taken(g.id) >= g.max_players {
            ErrorCode::GAME_FULL
        } else {
            return 0;
        };
        code.0
    }

    /// Players in the game, the seats tickets hold, and the seats a
    /// redeemed ticket took before the game counted them.
    fn seats_taken(&self, game: u32) -> u8 {
        let players = self.games.get(&game).map_or(0, |g| g.players);
        let held = self.tickets.values().filter(|t| t.game == game).count();
        let seating = self.seating.get(&game).copied().unwrap_or(0);
        players
            .saturating_add(held.min(255) as u8)
            .saturating_add(seating)
    }

    /// The list entry of `g` as `c` sees it.
    pub fn info(&self, c: &LobbyChar, g: &OpenGame) -> GameInfo {
        GameInfo {
            game_id: g.id,
            name: g.name.clone(),
            difficulty: g.difficulty,
            players: g.players,
            max_players: g.max_players,
            locked: g.locked(),
            hardcore: g.hardcore,
            expansion: g.expansion,
            joinable: self.joinable(c, g),
        }
    }

    /// The games `c` sees: every open game of its hardcore and expansion
    /// kind, those it can't join included (with the reason).
    pub fn list_for(&self, c: &LobbyChar) -> Vec<GameInfo> {
        let kind = kind_of(c);
        self.games
            .values()
            .filter(|g| (g.hardcore, g.expansion) == kind)
            .map(|g| self.info(c, g))
            .collect()
    }

    /// `ListGames`: the session listens from now on; returns its snapshot.
    pub fn subscribe(&mut self, session: u64, c: &LobbyChar) -> Vec<GameInfo> {
        self.listeners.insert(
            session,
            Listener {
                character: c.clone(),
            },
        );
        self.list_for(c)
    }

    /// `LeaveLobby`, a game joined, or the session ended.
    pub fn unsubscribe(&mut self, session: u64) {
        self.listeners.remove(&session);
    }

    pub fn listening(&self, session: u64) -> bool {
        self.listeners.contains_key(&session)
    }

    /// Every listener and what it sees of game `id` now: `Some(info)` (a new
    /// or changed entry), `None` when it can't see the game (another kind,
    /// or the game is gone).
    pub fn views_of(&self, id: u32) -> Vec<(u64, Option<GameInfo>)> {
        let g = self.games.get(&id);
        self.listeners
            .iter()
            .filter_map(|(&s, l)| {
                let kind = kind_of(&l.character);
                match g {
                    Some(g) if (g.hardcore, g.expansion) == kind => {
                        Some((s, Some(self.info(&l.character, g))))
                    }
                    Some(_) => None,
                    None => Some((s, None)),
                }
            })
            .collect()
    }

    /// The sessions listening (chat goes to all of them).
    pub fn listeners(&self) -> Vec<u64> {
        self.listeners.keys().copied().collect()
    }

    fn limits_of(&mut self, session: u64) -> &mut SessionLimits {
        let div = if self.limits.test_windows { 60 } else { 1 };
        self.session_limits
            .entry(session)
            .or_insert_with(|| SessionLimits::new(div))
    }

    fn not_busy(&self, session: u64, c: &LobbyChar) -> Result<(), Refusal> {
        if self.sessions_in_game.contains_key(&session) {
            return Err(ErrorCode::ALREADY_IN_GAME.into());
        }
        if self.playing.contains_key(&name_key(&c.name)) {
            return Err(ErrorCode::CHAR_IN_GAME.into());
        }
        Ok(())
    }

    /// `CreateGame`'s rules. On `Ok` the caller starts the game
    /// (`Games::start`) with the returned spec values and, once it runs,
    /// calls [`LobbyState::opened`].
    ///
    /// The check reserves neither the name nor a slot: the caller holds
    /// this state's lock from the check through [`LobbyState::opened`], so
    /// no other request can take either in between (the session glue does,
    /// it is all synchronous work between the two calls).
    pub fn check_create(
        &mut self,
        session: u64,
        c: &LobbyChar,
        new: NewGame,
        now: Instant,
    ) -> Result<OpenGame, Refusal> {
        let NewGame {
            name,
            password,
            difficulty,
            max_players,
        } = new;
        if dead_hardcore(c) {
            return Err(ErrorCode::DEAD_HARDCORE.into());
        }
        self.not_busy(session, c)?;
        self.limits_of(session).create.check(now).map_err(limited)?;
        let name = name.trim();
        if !game_name_ok(name) || password.len() > MAX_GAME_PASSWORD {
            return Err(ErrorCode::BAD_GAME_NAME.into());
        }
        if difficulty >= DIFFICULTIES || !status::difficulty_open(c.difficulties, difficulty) {
            return Err(ErrorCode::DIFFICULTY_LOCKED.into());
        }
        let key = name_key(name);
        if self.games.values().any(|g| name_key(&g.name) == key) {
            return Err(ErrorCode::GAME_NAME_TAKEN.into());
        }
        if self.games.len() >= self.limits.max_games {
            return Err(ErrorCode::TOO_MANY_GAMES.into());
        }
        self.limits_of(session).create.hit(now);
        let (hardcore, expansion) = kind_of(c);
        let max_players = match max_players {
            0 => self.limits.default_players,
            n => n.min(self.limits.players_cap),
        };
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        Ok(OpenGame {
            id,
            name: name.to_string(),
            password: password.to_string(),
            difficulty,
            hardcore,
            expansion,
            max_players,
            players: 0,
        })
    }

    /// The game of a successful [`LobbyState::check_create`] runs: it is
    /// listed from now on.
    pub fn opened(&mut self, g: OpenGame) {
        self.games.insert(g.id, g);
    }

    /// `JoinGame`'s rules for session `session` with character `c`.
    pub fn check_join(
        &mut self,
        session: u64,
        c: &LobbyChar,
        game_id: u32,
        password: &str,
        now: Instant,
    ) -> Result<(), Refusal> {
        if dead_hardcore(c) {
            return Err(ErrorCode::DEAD_HARDCORE.into());
        }
        self.not_busy(session, c)?;
        let g = self
            .games
            .get(&game_id)
            .ok_or(Refusal::from(ErrorCode::GAME_NOT_FOUND))?;
        match self.joinable(c, g) {
            0 => {}
            code => return Err(ErrorCode(code).into()),
        }
        if g.locked() {
            if password.is_empty() {
                return Err(ErrorCode::PASSWORD_NEEDED.into());
            }
            let right = password_eq(&g.password, password);
            let limits = self.limits_of(session);
            limits.wrong_password.check(now).map_err(limited)?;
            if !right {
                limits.wrong_password.hit(now);
                return Err(ErrorCode::WRONG_PASSWORD.into());
            }
        }
        Ok(())
    }

    /// Hold a seat for `c` in `game` with `ticket` (random, from the
    /// caller). The character counts as in the game from now on.
    ///
    /// The seat is re-checked here: the game must still exist and have room
    /// (the rules in [`LobbyState::check_join`] ran under an earlier lock
    /// hold, and a game can fill in between).
    pub fn issue(
        &mut self,
        session: u64,
        c: &LobbyChar,
        game: u32,
        ticket: [u8; 16],
        now: Instant,
    ) -> Result<(), Refusal> {
        let g = self
            .games
            .get(&game)
            .ok_or(Refusal::from(ErrorCode::GAME_NOT_FOUND))?;
        if self.seats_taken(game) >= g.max_players {
            return Err(ErrorCode::GAME_FULL.into());
        }
        self.not_busy(session, c)?;
        self.tickets.insert(
            ticket,
            Ticket {
                session,
                game,
                character: c.name.clone(),
                expires: now + TICKET_TTL,
            },
        );
        self.playing.insert(name_key(&c.name), (game, session));
        self.sessions_in_game.insert(session, game);
        self.listeners.remove(&session);
        Ok(())
    }

    /// `Attach` mode 0: the ticket once, within 30 s, on the session it was
    /// given to. A redeemed seat stays counted ([`LobbyState::seating`])
    /// until the game's next `Players`.
    pub fn redeem(
        &mut self,
        session: u64,
        ticket: [u8; 16],
        now: Instant,
    ) -> Result<Ticket, RedeemError> {
        match self.tickets.get(&ticket) {
            Some(t) if t.session == session && now < t.expires => {
                let t = self.tickets.remove(&ticket).expect("checked above");
                let seats = self.seating.entry(t.game).or_insert(0);
                *seats = seats.saturating_add(1);
                Ok(t)
            }
            Some(t) if now >= t.expires => {
                let t = self.tickets.remove(&ticket).expect("checked above");
                self.release(t.session, &t.character);
                Err(RedeemError {
                    refusal: ErrorCode::TICKET_INVALID.into(),
                    freed: Some(t.game),
                })
            }
            _ => Err(Refusal::from(ErrorCode::TICKET_INVALID).into()),
        }
    }

    /// The ticket, while it is issued.
    pub fn ticket(&self, t: &[u8; 16]) -> Option<&Ticket> {
        self.tickets.get(t)
    }

    /// Drop expired tickets (their seats free up); returns them with
    /// their games, whose seat counts changed.
    pub fn expire(&mut self, now: Instant) -> Vec<([u8; 16], u32)> {
        let gone: Vec<[u8; 16]> = self
            .tickets
            .iter()
            .filter(|(_, t)| now >= t.expires)
            .map(|(k, _)| *k)
            .collect();
        let mut out = Vec::new();
        for k in gone {
            if let Some(t) = self.tickets.remove(&k) {
                self.release(t.session, &t.character);
                out.push((k, t.game));
            }
        }
        out
    }

    fn release(&mut self, session: u64, character: &str) {
        self.playing.remove(&name_key(character));
        self.sessions_in_game.remove(&session);
    }

    /// `GameEvent::Players`: the game's own count now includes every seat
    /// it seated, so the pending seating counts go.
    pub fn players(&mut self, game: u32, players: u8) {
        if let Some(g) = self.games.get_mut(&game) {
            g.players = players;
        }
        self.seating.remove(&game);
    }

    /// `GameEvent::Left`: the character and the session that took it into
    /// the game are free again.
    pub fn left(&mut self, character: &str) {
        if let Some((_, session)) = self.playing.remove(&name_key(character)) {
            self.sessions_in_game.remove(&session);
        }
    }

    /// `GameEvent::Closed`: the game leaves the list; its tickets and
    /// players are free.
    pub fn closed(&mut self, game: u32) {
        self.games.remove(&game);
        self.tickets.retain(|_, t| t.game != game);
        self.playing.retain(|_, (g, _)| *g != game);
        self.sessions_in_game.retain(|_, g| *g != game);
        self.seating.remove(&game);
    }

    /// The session ended: it stops listening, its tickets go. A character
    /// already in a game stays there until the game says it left.
    pub fn session_ended(&mut self, session: u64) {
        self.listeners.remove(&session);
        self.session_limits.remove(&session);
        let held: Vec<[u8; 16]> = self
            .tickets
            .iter()
            .filter(|(_, t)| t.session == session)
            .map(|(k, _)| *k)
            .collect();
        for k in held {
            if let Some(t) = self.tickets.remove(&k) {
                self.release(session, &t.character);
            }
        }
    }

    /// Whether the character (name key) is in a game or holds a ticket.
    pub fn character_in_game(&self, key: &str) -> bool {
        self.playing.contains_key(key)
    }

    /// `Chat`: the session's rate limit.
    pub fn check_chat(&mut self, session: u64, now: Instant) -> Result<(), Refusal> {
        let w = &mut self.limits_of(session).chat;
        w.check(now).map_err(limited)?;
        w.hit(now);
        Ok(())
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
