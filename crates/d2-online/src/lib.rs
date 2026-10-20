//! The online client session, shared by the game and the multiplayer test
//! bots so both speak exactly one implementation of the protocol.
//!
//! - [`connect`]: the QUIC connection, the pinned certificate, the
//!   handshake and the control stream ([`Session`]).
//! - [`game`]: the game stream of the game the session joined
//!   ([`GameLink`]).
//! - The account and lobby requests are methods on [`Session`], each in
//!   its own module.
//!
//! Everything is synchronous: sends queue and return, polls return what
//! has arrived. Only connecting and the request helpers wait, each for at
//! most its timeout, so a game runs them off its frame loop. No game code
//! lives here: the game applies what a [`GameLink`] hands it.

pub mod account;
pub mod connect;
pub mod game;
pub mod lobby;

pub use account::SessionEvent;
pub use connect::{ClientInfo, ConnectOptions, ServerSpec, Session};
pub use d2_net::msg::ErrorCode;
pub use d2_net::wire::{ResumeState, SessionWelcome};
pub use d2_quic::{fingerprint_hex, parse_fingerprint, ServerTrust};
pub use game::{FrameAssembler, GameLink, ServerFrame};
pub use lobby::{GameList, JoinOk};

use d2_net::NetError;

/// Why an online request failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OnlineError {
    /// The server could not be reached, or it is not the server the list
    /// names (its certificate does not match the pinned fingerprint).
    #[error("could not connect to the server: {0}")]
    Connect(String),
    /// The server refused the session or the game slot (the handshake's or
    /// the attach's `Refused`).
    #[error("{text}")]
    Refused { code: ErrorCode, text: String },
    /// The server answered a request with `ERROR`: the request's kind, the
    /// code, how long to wait before trying again, the server's words.
    #[error("{text}")]
    Server {
        request_kind: u8,
        code: ErrorCode,
        wait_s: u32,
        text: String,
    },
    /// The connection or the stream is gone.
    #[error("the connection to the server was lost ({0})")]
    Lost(NetError),
    /// The server broke the protocol; the connection is closed.
    #[error("the server sent something this game does not understand")]
    Malformed,
    /// No answer in time.
    #[error("the server did not answer in time")]
    TimedOut,
}

impl OnlineError {
    /// The server's error code, for a refusal or a failed request.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Refused { code, .. } | Self::Server { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// How long the server said to wait before trying again (0 when it
    /// didn't).
    pub fn wait_s(&self) -> u32 {
        match self {
            Self::Server { wait_s, .. } => *wait_s,
            _ => 0,
        }
    }

    /// The server's own words for a refusal or a failed request; this
    /// client's description otherwise.
    pub fn text(&self) -> String {
        match self {
            Self::Refused { text, .. } | Self::Server { text, .. } => text.clone(),
            other => other.to_string(),
        }
    }

    /// What the player reads: this game's own words for a code it knows,
    /// the server's words for one it doesn't, and how long to wait when the
    /// server said so.
    pub fn message(&self) -> String {
        let words = |code: ErrorCode, text: &str| {
            if ErrorCode::ALL.contains(&code) {
                code.text().to_owned()
            } else {
                text.to_owned()
            }
        };
        match self {
            Self::Refused { code, text } => words(*code, text),
            Self::Server {
                code, wait_s, text, ..
            } => {
                let w = words(*code, text);
                match *wait_s {
                    0 => w,
                    s if s < 120 => format!("{w} Try again in {s} seconds."),
                    s => format!("{w} Try again in {} minutes.", s.div_ceil(60)),
                }
            }
            Self::Connect(_) => {
                "Could not reach the server. Check your internet connection and try again."
                    .to_owned()
            }
            Self::Lost(_) => "The connection to the server was lost.".to_owned(),
            Self::Malformed => {
                "The server sent something this game does not understand. Update the game."
                    .to_owned()
            }
            Self::TimedOut => "The server did not answer in time. Try again.".to_owned(),
        }
    }
}

impl From<d2_net::wire::Refused> for OnlineError {
    fn from(r: d2_net::wire::Refused) -> Self {
        Self::Refused {
            code: r.code,
            text: r.text,
        }
    }
}

impl From<d2_net::msg::ErrorMsg> for OnlineError {
    fn from(e: d2_net::msg::ErrorMsg) -> Self {
        Self::Server {
            request_kind: e.request_kind,
            code: e.code,
            wait_s: e.wait_s,
            text: e.text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_read_in_the_games_words_and_unknown_in_the_servers() {
        let known = OnlineError::Refused {
            code: ErrorCode::BUILD,
            text: "server words".into(),
        };
        assert_eq!(known.message(), ErrorCode::BUILD.text());
        let unknown = OnlineError::Refused {
            code: ErrorCode(999),
            text: "A new rule.".into(),
        };
        assert_eq!(unknown.message(), "A new rule.");
    }

    #[test]
    fn a_wait_is_said_in_seconds_or_minutes() {
        let e = |wait_s| OnlineError::Server {
            request_kind: 0x21,
            code: ErrorCode::RATE_LIMITED,
            wait_s,
            text: String::new(),
        };
        assert_eq!(e(0).message(), ErrorCode::RATE_LIMITED.text());
        assert!(e(30).message().ends_with("Try again in 30 seconds."));
        assert!(e(900).message().ends_with("Try again in 15 minutes."));
    }
}
