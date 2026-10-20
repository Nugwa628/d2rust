//! What the online screens send to the server and what comes back, in the
//! screens' own terms. The screens never touch the network: they push
//! [`Request`]s into an [`OnlineLink`] and read [`Event`]s from it once per
//! tick. The game's link (the `online` feature) maps these onto the control
//! stream messages of `docs/plans/server/multiplayer-contracts.md` (sections
//! 3.4-3.8); tests and offscreen captures use [`super::fake::FakeServer`].
//!
//! A `Request::CreateGame` carries no player count: the link sends
//! `max_players` 0, the server's default (contracts section 3.6).

pub use d2_net::msg::ErrorCode;

/// One server of `servers.toml`, as the login screen lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerEntry {
    pub id: String,
    pub name: String,
    /// `host:port`, shown under the name.
    pub address: String,
}

/// An online character (the contract's `CharSummary`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharInfo {
    pub name: String,
    /// 0-6.
    pub class: u8,
    pub level: u8,
    /// The save status word: hardcore, dead, expansion, progression.
    pub status: u16,
    /// Unlocked difficulties: bit 0 Normal, bit 1 Nightmare, bit 2 Hell.
    pub difficulties: u8,
    /// The save header's appearance bytes: 16 components, then 16 colours.
    pub look: Vec<u8>,
    /// Unix seconds, 0 if never played.
    pub last_played: u64,
}

/// One open game in the lobby's list (the contract's `GameInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameInfo {
    pub id: u32,
    pub name: String,
    /// 0 Normal, 1 Nightmare, 2 Hell.
    pub difficulty: u8,
    pub players: u8,
    pub max_players: u8,
    /// Has a password.
    pub locked: bool,
    pub hardcore: bool,
    pub expansion: bool,
    /// 0: the selected character can join; else the error code why not.
    pub joinable: u16,
}

/// A game the server let the character into: the host starts the game
/// stream with the ticket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Joined {
    pub game_id: u32,
    pub ticket: [u8; 16],
    pub difficulty: u8,
    /// The game's name, for the loading screen.
    pub name: String,
}

/// A failed request (the contract's `ERROR`, `Refused`, or the connection
/// failing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// The error code (section 3.8); `ErrorCode(0)` for a connection
    /// failure of the client's own (no route, timeout, certificate
    /// mismatch).
    pub code: ErrorCode,
    /// Seconds to wait, for rate limits and locks.
    pub wait_s: u32,
    /// The server's English text (shown for a code the client does not know).
    pub text: String,
}

/// What a screen asks of the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Open the connection and do the handshake with server `index` of the
    /// list.
    Connect {
        index: usize,
    },
    Register {
        account: String,
        password: String,
    },
    Login {
        account: String,
        password: String,
    },
    CharList,
    CharCreate {
        name: String,
        class: u8,
        hardcore: bool,
        expansion: bool,
    },
    /// `password`: the account password, to confirm.
    CharDelete {
        name: String,
        password: String,
    },
    CharSelect {
        name: String,
    },
    /// Subscribe to the game list (a snapshot, then updates).
    ListGames,
    CreateGame {
        name: String,
        password: String,
        difficulty: u8,
    },
    JoinGame {
        game_id: u32,
        password: String,
    },
    /// Stop the list updates (back to character select).
    LeaveLobby,
    Chat {
        text: String,
    },
    /// End the session and close the connection.
    Logout,
}

/// What the server answered or sent on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Connected {
        server_name: String,
        registration_open: bool,
    },
    LoggedIn {
        account: String,
        max_characters: u8,
    },
    Chars(Vec<CharInfo>),
    CharCreated(CharInfo),
    CharDeleted(String),
    CharSelected(CharInfo),
    /// A list snapshot starts: the screen drops its list.
    GameListBegin,
    /// A new or changed game.
    Game(GameInfo),
    GameListEnd,
    GameGone(u32),
    JoinOk(Joined),
    ChatLine {
        from: String,
        text: String,
    },
    /// A request failed; the session goes on.
    Error(Failure),
    /// The connection is gone (refused, lost, taken over, logged out).
    Disconnected(Failure),
}

/// The connection the screens talk through.
pub trait OnlineLink {
    fn send(&mut self, r: Request);
    /// The next event, if one arrived.
    fn poll(&mut self) -> Option<Event>;
}

/// No link: every request fails at once (a build without online play, or a
/// host that wired none).
#[derive(Debug, Default)]
pub struct NoLink {
    out: std::collections::VecDeque<Event>,
}

impl OnlineLink for NoLink {
    fn send(&mut self, _r: Request) {
        self.out.push_back(Event::Disconnected(Failure {
            code: ErrorCode(0),
            wait_s: 0,
            text: "Online play is not part of this build.".into(),
        }));
    }

    fn poll(&mut self) -> Option<Event> {
        self.out.pop_front()
    }
}

/// The message box text for a failure: the client's own sentence for a
/// known code (the rule the player broke, and what to do), the server's
/// text for an unknown one. Rate limits and locks say how long to wait.
pub fn failure_text(f: &Failure) -> String {
    let wait = || match f.wait_s {
        0 => String::new(),
        s if s < 60 => format!(" Try again in {s} seconds."),
        s => format!(" Try again in {} minutes.", s.div_ceil(60)),
    };
    let own = match f.code {
        ErrorCode(0) if f.text.is_empty() => "Could not reach the server.",
        ErrorCode(0) => return f.text.clone(),
        ErrorCode::WIRE_VERSION | ErrorCode::PROTOCOL | ErrorCode::TICK_RATE => {
            "This server runs another version of D2RUST. Update the game through the launcher."
        }
        // The server's text names both versions.
        ErrorCode::BUILD if !f.text.is_empty() => return f.text.clone(),
        ErrorCode::BUILD => "This server runs another version of D2RUST. Update the game through the launcher.",
        ErrorCode::WRONG_GAME => "This server runs another game.",
        ErrorCode::SERVER_FULL => "The server is full. Try again later.",
        ErrorCode::UNKNOWN_RESUME => "Your session has ended. Log in again.",
        ErrorCode::NOT_SUPPORTED => "The server does not support this yet.",
        ErrorCode::MALFORMED => "The connection broke (a message did not decode). Log in again.",
        ErrorCode::ACCOUNT_NAME_TAKEN => "That account name is already taken.",
        ErrorCode::BAD_ACCOUNT_NAME if !f.text.is_empty() => return f.text.clone(),
        ErrorCode::BAD_ACCOUNT_NAME => {
            "Account names are 2-15 letters, digits, - or _, and start with a letter."
        }
        ErrorCode::BAD_PASSWORD => "Passwords are 8 to 64 characters long.",
        ErrorCode::WRONG_LOGIN => "Wrong account name or password.",
        ErrorCode::RATE_LIMITED => return format!("Too many attempts.{}", wait()),
        ErrorCode::ACCOUNT_LOCKED => {
            return format!("This account is locked after too many failed logins.{}", wait())
        }
        ErrorCode::ACCOUNT_DISABLED => "This account is disabled.",
        ErrorCode::NOT_LOGGED_IN => "You are not logged in. Log in again.",
        ErrorCode::REGISTRATION_CLOSED => "This server does not take new accounts.",
        ErrorCode::SESSION_TAKEN => "Someone logged in to this account from another place.",
        ErrorCode::CHAR_NAME_TAKEN => "That character name is already taken.",
        ErrorCode::BAD_CHAR_NAME if !f.text.is_empty() => return f.text.clone(),
        ErrorCode::BAD_CHAR_NAME => "That character name is not allowed.",
        ErrorCode::CHAR_LIMIT => "This account has no free character slot.",
        ErrorCode::UNKNOWN_CHAR => "That character is not on this account.",
        ErrorCode::CHAR_IN_GAME => "That character is in a game right now.",
        ErrorCode::CHAR_MISMATCH => "The game refused the character.",
        ErrorCode::DEAD_HARDCORE => "You cannot create or join games with a dead hardcore character.",
        ErrorCode::NO_CHAR_SELECTED => "Choose a character first.",
        ErrorCode::GAME_NAME_TAKEN => "A game with that name is already open.",
        ErrorCode::BAD_GAME_NAME => "Game names are 1-15 letters, digits, spaces, - or _.",
        ErrorCode::GAME_NOT_FOUND => "That game is gone.",
        ErrorCode::GAME_FULL => "That game is full.",
        ErrorCode::PASSWORD_NEEDED => "That game needs a password.",
        ErrorCode::WRONG_PASSWORD => "Wrong game password.",
        ErrorCode::DIFFICULTY_LOCKED => "Your character has not unlocked that difficulty.",
        ErrorCode::WRONG_GAME_KIND => {
            "That game is for another kind of character (hardcore or softcore, expansion or classic)."
        }
        ErrorCode::TOO_MANY_GAMES => "The server runs as many games as it can. Try again later.",
        ErrorCode::ALREADY_IN_GAME => "You are already in a game.",
        ErrorCode::GAME_CLOSED => "The game closed.",
        ErrorCode::TICKET_INVALID => "The join expired. Join the game again.",
        _ if !f.text.is_empty() => return f.text.clone(),
        _ => return format!("The server refused (error {}).", f.code.0),
    };
    own.to_string()
}

/// A difficulty's name as the lobby shows it.
pub fn difficulty_name(d: u8) -> &'static str {
    match d {
        0 => "Normal",
        1 => "Nightmare",
        _ => "Hell",
    }
}
