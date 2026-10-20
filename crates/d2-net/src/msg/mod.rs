//! The control-stream messages of an online session: their field codec,
//! their kind numbers and the error answer every request can get.
//!
//! An online connection has a control stream (the handshake, then
//! accounts, characters and the lobby for the whole session) and, while
//! the player is in a game, a game stream (wire v2 packets). Both use the
//! frames of [`crate::wire`]; this module is what goes inside the control
//! stream's frame bodies. The layouts are fixed in
//! `docs/plans/server/multiplayer-contracts.md` section 3.
//!
//! - **Fields** are little-endian. A string is a byte length then UTF-8
//!   with no control characters; `bytes8` is a byte length then raw bytes;
//!   a `bool` is one byte, 0 or 1; an optional field is a byte 0, or 1 then
//!   the field. A message with bytes left after its last field is
//!   malformed. [`Writer`] and [`Reader`] are the only codec.
//! - **Kinds** (the first body byte) are split in ranges, one per area
//!   ([`kind`]), so each area adds messages without touching the others.
//! - **Errors:** a request that fails is answered with [`ErrorMsg`]: the
//!   request's kind, an [`ErrorCode`], a wait in seconds for limits, and an
//!   English text. The client shows its own words for a code it knows, and
//!   the server's text for one it doesn't.

use crate::NetError;

pub mod account;
pub mod lobby;

/// Frame kinds of the control stream. Kinds 1-3 (`HELLO`, `WELCOME`,
/// `REFUSED`) and the control frames `PING` and `BYE` are
/// [`crate::wire::kind`]'s; the game stream's kinds are there too.
pub mod kind {
    use std::ops::RangeInclusive;

    /// S→C: a request failed ([`super::ErrorMsg`]).
    pub const ERROR: u8 = 0x10;
    /// Kinds of this module's own messages (the error answer and later
    /// shared ones).
    pub const SESSION: RangeInclusive<u8> = 0x10..=0x1F;
    /// Account and character messages (`msg::account`).
    pub const ACCOUNT: RangeInclusive<u8> = 0x20..=0x3F;
    /// Lobby messages: game list, create, join, chat (`msg::lobby`).
    pub const LOBBY: RangeInclusive<u8> = 0x40..=0x5F;
    /// Kept for a mod game's own lobby messages.
    pub const MOD: RangeInclusive<u8> = 0x60..=0x6F;
}

/// Builds one message body.
#[derive(Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// A body that starts with the message's kind byte.
    pub fn new(kind: u8) -> Self {
        Self { buf: vec![kind] }
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(u8::from(v))
    }

    /// A fixed-size byte field (tokens, tickets).
    pub fn array<const N: usize>(&mut self, v: &[u8; N]) -> &mut Self {
        self.buf.extend_from_slice(v);
        self
    }

    /// A string of at most 255 bytes. A longer one is cut at the last
    /// character that fits: a sender keeps its strings inside the limits of
    /// the message, so a cut string is refused by the receiver's own limit.
    pub fn str8(&mut self, s: &str) -> &mut Self {
        let mut n = s.len().min(u8::MAX as usize);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.buf.push(n as u8);
        self.buf.extend_from_slice(&s.as_bytes()[..n]);
        self
    }

    /// Raw bytes, at most 255 (longer ones are cut).
    pub fn bytes8(&mut self, b: &[u8]) -> &mut Self {
        let n = b.len().min(u8::MAX as usize);
        self.buf.push(n as u8);
        self.buf.extend_from_slice(&b[..n]);
        self
    }

    /// An optional field: 0, or 1 then what `put` writes.
    pub fn opt<T>(&mut self, v: Option<&T>, put: impl FnOnce(&mut Self, &T)) -> &mut Self {
        match v {
            None => self.u8(0),
            Some(v) => {
                self.u8(1);
                put(self, v);
                self
            }
        }
    }

    /// The finished body.
    pub fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

/// Reads one message body's fields, after its kind byte. Every read fails
/// with [`NetError::Malformed`] on a short body or a value outside its
/// field's rule.
#[derive(Debug)]
pub struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    /// A reader over `body`: returns the kind byte and the reader for the
    /// fields after it.
    pub fn new(body: &'a [u8]) -> Result<(u8, Self), NetError> {
        let (&kind, rest) = body.split_first().ok_or(NetError::Malformed)?;
        Ok((kind, Self { rest }))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], NetError> {
        if self.rest.len() < n {
            return Err(NetError::Malformed);
        }
        let (head, rest) = self.rest.split_at(n);
        self.rest = rest;
        Ok(head)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], NetError> {
        let b = self.take(N)?;
        b.try_into().map_err(|_| NetError::Malformed)
    }

    pub fn u8(&mut self) -> Result<u8, NetError> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u16(&mut self) -> Result<u16, NetError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, NetError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, NetError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn bool(&mut self) -> Result<bool, NetError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(NetError::Malformed),
        }
    }

    /// A string of at most `max` bytes, valid UTF-8 with no control
    /// characters.
    pub fn str8(&mut self, max: usize) -> Result<String, NetError> {
        let n = self.u8()? as usize;
        if n > max {
            return Err(NetError::Malformed);
        }
        let s = std::str::from_utf8(self.take(n)?).map_err(|_| NetError::Malformed)?;
        if s.chars().any(char::is_control) {
            return Err(NetError::Malformed);
        }
        Ok(s.to_owned())
    }

    /// Raw bytes, at most `max`.
    pub fn bytes8(&mut self, max: usize) -> Result<Vec<u8>, NetError> {
        let n = self.u8()? as usize;
        if n > max {
            return Err(NetError::Malformed);
        }
        Ok(self.take(n)?.to_vec())
    }

    /// An optional field read by `get`.
    pub fn opt<T>(
        &mut self,
        get: impl FnOnce(&mut Self) -> Result<T, NetError>,
    ) -> Result<Option<T>, NetError> {
        match self.u8()? {
            0 => Ok(None),
            1 => get(self).map(Some),
            _ => Err(NetError::Malformed),
        }
    }

    /// Ends the message: fails if bytes are left.
    pub fn finish(self) -> Result<(), NetError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(NetError::Malformed)
        }
    }
}

/// Why a request failed or a connection was refused (contracts section
/// 3.8). A code never changes meaning; a client that doesn't know a code
/// shows the server's text, so new codes need no new client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ErrorCode(pub u16);

macro_rules! error_codes {
    ($($name:ident = $n:literal, $contract:ident, $text:literal;)*) => {
        impl ErrorCode {
            $(pub const $name: Self = Self($n);)*

            /// Every code this build knows.
            pub const ALL: &'static [Self] = &[$(Self::$name),*];

            /// The code's name in the contracts' table (`WrongLogin`), for
            /// logs and test reports.
            pub fn name(self) -> &'static str {
                match self.0 {
                    $($n => stringify!($contract),)*
                    _ => "Unknown",
                }
            }

            /// The English words for the code, which the server sends as the
            /// text when it has nothing more specific to say.
            pub fn text(self) -> &'static str {
                match self.0 {
                    $($n => $text,)*
                    _ => "Something went wrong on the server.",
                }
            }
        }
    };
}

error_codes! {
    WIRE_VERSION = 1, WireVersion, "This is not a D2RUST server of this version. Update the game.";
    PROTOCOL = 2, Protocol, "The server runs another protocol version. Update the game.";
    BUILD = 3, Build, "The server runs another game version. Update through the launcher.";
    TICK_RATE = 4, TickRate, "The server runs at another tick rate. Update the game.";
    WRONG_GAME = 5, WrongGame, "The server runs another game.";
    SERVER_FULL = 6, ServerFull, "The server is full. Try again later.";
    UNKNOWN_RESUME = 7, UnknownResume, "The server no longer holds your session.";
    NOT_SUPPORTED = 8, NotSupported, "The server doesn't support that yet.";
    MALFORMED = 9, Malformed, "The server couldn't read a message.";
    ACCOUNT_NAME_TAKEN = 20, AccountNameTaken, "That account name is taken.";
    BAD_ACCOUNT_NAME = 21, BadAccountName, "That account name isn't allowed.";
    BAD_PASSWORD = 22, BadPassword, "The password must be 8 to 64 bytes long.";
    WRONG_LOGIN = 23, WrongLogin, "Wrong account name or password.";
    RATE_LIMITED = 24, RateLimited, "Too many attempts. Wait a moment and try again.";
    ACCOUNT_LOCKED = 25, AccountLocked, "Too many failed logins. The account is locked for a while.";
    ACCOUNT_DISABLED = 26, AccountDisabled, "This account is disabled.";
    NOT_LOGGED_IN = 27, NotLoggedIn, "You are not logged in.";
    REGISTRATION_CLOSED = 28, RegistrationClosed, "This server doesn't take new accounts.";
    SESSION_TAKEN = 29, SessionTaken, "Someone logged in to this account elsewhere.";
    CHAR_NAME_TAKEN = 40, CharNameTaken, "That character name is taken.";
    BAD_CHAR_NAME = 41, BadCharName, "That character name isn't allowed.";
    CHAR_LIMIT = 42, CharLimit, "The account has as many characters as it may.";
    UNKNOWN_CHAR = 43, UnknownChar, "That character isn't on this account.";
    CHAR_IN_GAME = 44, CharInGame, "That character is in a game.";
    CHAR_MISMATCH = 45, CharMismatch, "The game was joined with another character.";
    DEAD_HARDCORE = 46, DeadHardcore, "You cannot create or join games with a dead hardcore character.";
    NO_CHAR_SELECTED = 47, NoCharSelected, "Select a character first.";
    EXPANSION_CLASS = 48, ExpansionClass, "Druids and Assassins are expansion characters.";
    GAME_NAME_TAKEN = 60, GameNameTaken, "A game with that name exists already.";
    BAD_GAME_NAME = 61, BadGameName, "That game name isn't allowed.";
    GAME_NOT_FOUND = 62, GameNotFound, "That game doesn't exist any more.";
    GAME_FULL = 63, GameFull, "That game is full.";
    PASSWORD_NEEDED = 64, PasswordNeeded, "That game needs a password.";
    WRONG_PASSWORD = 65, WrongPassword, "Wrong game password.";
    DIFFICULTY_LOCKED = 66, DifficultyLocked, "Your character hasn't unlocked that difficulty.";
    WRONG_GAME_KIND = 67, WrongGameKind, "Your character can't join that kind of game.";
    TOO_MANY_GAMES = 68, TooManyGames, "The server runs as many games as it can. Try again later.";
    ALREADY_IN_GAME = 69, AlreadyInGame, "You are in a game already.";
    GAME_CLOSED = 70, GameClosed, "The game closed.";
    TICKET_INVALID = 71, TicketInvalid, "The join expired. Join the game again.";
    OTHER = 99, Other, "Something went wrong on the server.";
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name(), self.0)
    }
}

/// Longest error text the server sends.
pub const MAX_ERROR_TEXT: usize = 200;

/// S→C [`kind::ERROR`]: the answer to a request that failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorMsg {
    /// The kind of the request this answers.
    pub request_kind: u8,
    pub code: ErrorCode,
    /// How long to wait before trying again, for [`ErrorCode::RATE_LIMITED`]
    /// and [`ErrorCode::ACCOUNT_LOCKED`]; 0 otherwise.
    pub wait_s: u32,
    /// The server's words; the client shows its own for a known code.
    pub text: String,
}

impl ErrorMsg {
    /// An error with the code's own words.
    pub fn new(request_kind: u8, code: ErrorCode) -> Self {
        Self {
            request_kind,
            code,
            wait_s: 0,
            text: code.text().to_owned(),
        }
    }

    pub fn body(&self) -> Vec<u8> {
        Writer::new(kind::ERROR)
            .u8(self.request_kind)
            .u16(self.code.0)
            .u32(self.wait_s)
            .str8(&self.text)
            .finish()
    }

    /// Reads an `ERROR` body (kind byte included).
    pub fn parse(body: &[u8]) -> Result<Self, NetError> {
        let (k, mut r) = Reader::new(body)?;
        if k != kind::ERROR {
            return Err(NetError::Malformed);
        }
        let m = Self {
            request_kind: r.u8()?,
            code: ErrorCode(r.u16()?),
            wait_s: r.u32()?,
            text: r.str8(MAX_ERROR_TEXT)?,
        };
        r.finish()?;
        Ok(m)
    }
}

#[cfg(test)]
mod tests;
