//! The account side of an online session: register, log in and out, and
//! the account's online characters (contracts section 3.5). Each request
//! goes on the control stream; a refusal comes back as
//! [`OnlineError::Server`] with the server's code, wait and words, and
//! messages that arrive meanwhile (lobby updates, chat) stay for
//! [`Session::poll`].
//!
//! Two shapes: the blocking methods below (one request and its answer,
//! for simple callers and tests) and the non-blocking
//! [`Session::send_request`] + [`Session::poll_event`] pair (the bots,
//! 64 sessions on one thread, and the game screens, one poll per tick).

use std::time::Duration;

use d2_net::msg::account::{kind, AccountEvent, AccountRequest, CharSummary};
use d2_net::msg::{self, ErrorCode, ErrorMsg};

use crate::{OnlineError, Session};

/// How long an account request waits for its answer. A failed login is
/// answered after the server's fixed delay, and hashing takes a moment,
/// so this is generous.
pub const ACCOUNT_WAIT: Duration = Duration::from_secs(10);

/// The account a session is logged in to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoggedIn {
    /// The name as stored on the server, with its case.
    pub account: String,
    /// How many characters the account may have.
    pub max_characters: u8,
}

/// One account event, decoded, for callers that must not wait: the bots
/// (64 sessions on one thread) and the game screens (one poll per tick).
/// [`Session::poll_event`] returns it without blocking; the blocking
/// helpers above are built for simple callers and tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// The session is logged in (after `Register` or `Login`).
    LoggedIn {
        account: String,
        max_characters: u8,
    },
    /// The account's characters, oldest first.
    Chars(Vec<CharSummary>),
    CharCreated(CharSummary),
    CharDeleted {
        name: String,
    },
    CharSelected(CharSummary),
    /// A request failed; the session goes on.
    Error {
        request_kind: u8,
        code: ErrorCode,
        wait_s: u32,
        text: String,
    },
    /// The connection is gone (lost or broken); nothing more will come.
    Disconnected(OnlineError),
}

impl SessionEvent {
    fn of_account(ev: AccountEvent) -> Self {
        match ev {
            AccountEvent::LoggedIn {
                account,
                max_characters,
            } => Self::LoggedIn {
                account,
                max_characters,
            },
            AccountEvent::CharList(chars) => Self::Chars(chars),
            AccountEvent::CharCreated(c) => Self::CharCreated(c),
            AccountEvent::CharDeleted { name } => Self::CharDeleted { name },
            AccountEvent::CharSelected(c) => Self::CharSelected(c),
        }
    }

    fn of_error(e: ErrorMsg) -> Self {
        Self::Error {
            request_kind: e.request_kind,
            code: e.code,
            wait_s: e.wait_s,
            text: e.text,
        }
    }
}

/// The highest account request kind (contracts 3.5).
const LAST_ACCOUNT_KIND: u8 = kind::CHAR_SELECTED;

impl Session {
    /// Sends one account request and returns at once: its answer (or its
    /// `ERROR`) comes from [`Session::poll_event`].
    pub fn send_request(&mut self, req: AccountRequest) -> Result<(), OnlineError> {
        self.send(req.body())
    }

    /// The next account event that has arrived, or `None`; never blocks.
    /// An `ERROR` for an account request is [`SessionEvent::Error`]. A
    /// body for another part of the protocol (lobby, chat) is left at the
    /// front of the queue for that part's decoder and `None` is returned,
    /// so a caller in the lobby drains those bodies with [`Session::poll`]
    /// (or the lobby's own poller) as well.
    pub fn poll_event(&mut self) -> Option<SessionEvent> {
        let body = match self.poll() {
            Ok(Some(b)) => b,
            Ok(None) => return None,
            Err(e) => return Some(SessionEvent::Disconnected(e)),
        };
        match body.first().copied() {
            Some(msg::kind::ERROR) => match ErrorMsg::parse(&body) {
                Ok(e) if (kind::REGISTER..=LAST_ACCOUNT_KIND).contains(&e.request_kind) => {
                    Some(SessionEvent::of_error(e))
                }
                Ok(_) => {
                    self.requeue(body);
                    None
                }
                Err(_) => Some(SessionEvent::Disconnected(OnlineError::Malformed)),
            },
            Some(k) if (kind::LOGGED_IN..=LAST_ACCOUNT_KIND).contains(&k) => {
                match AccountEvent::parse(&body) {
                    Ok(ev) => Some(SessionEvent::of_account(ev)),
                    Err(_) => Some(SessionEvent::Disconnected(OnlineError::Malformed)),
                }
            }
            Some(_) => {
                self.requeue(body);
                None
            }
            None => Some(SessionEvent::Disconnected(OnlineError::Malformed)),
        }
    }

    fn account_request(
        &mut self,
        req: &AccountRequest,
        answer: u8,
    ) -> Result<AccountEvent, OnlineError> {
        let body = self.request(req.body(), &[answer], ACCOUNT_WAIT)?;
        AccountEvent::parse(&body).map_err(|_| OnlineError::Malformed)
    }

    fn logged_in(ev: AccountEvent) -> Result<LoggedIn, OnlineError> {
        match ev {
            AccountEvent::LoggedIn {
                account,
                max_characters,
            } => Ok(LoggedIn {
                account,
                max_characters,
            }),
            _ => Err(OnlineError::Malformed),
        }
    }

    /// Makes an account and logs the session in to it.
    pub fn register(&mut self, account: &str, password: &str) -> Result<LoggedIn, OnlineError> {
        let req = AccountRequest::Register {
            account: account.into(),
            password: password.into(),
        };
        Self::logged_in(self.account_request(&req, kind::LOGGED_IN)?)
    }

    /// Logs the session in. A login elsewhere later takes the account over
    /// (this session then gets `SessionTaken` and is closed).
    pub fn login(&mut self, account: &str, password: &str) -> Result<LoggedIn, OnlineError> {
        let req = AccountRequest::Login {
            account: account.into(),
            password: password.into(),
        };
        Self::logged_in(self.account_request(&req, kind::LOGGED_IN)?)
    }

    /// Logs out: the server ends the session, so the session is closed.
    pub fn logout(mut self) {
        let _ = self.send(AccountRequest::Logout.body());
        self.close();
    }

    /// The account's characters, oldest first.
    pub fn characters(&mut self) -> Result<Vec<CharSummary>, OnlineError> {
        match self.account_request(&AccountRequest::CharList, kind::CHAR_LIST_ANSWER)? {
            AccountEvent::CharList(chars) => Ok(chars),
            _ => Err(OnlineError::Malformed),
        }
    }

    /// Creates a character on the account. It is made in its first game.
    pub fn create_character(
        &mut self,
        name: &str,
        class: u8,
        hardcore: bool,
        expansion: bool,
    ) -> Result<CharSummary, OnlineError> {
        let req = AccountRequest::CharCreate {
            name: name.into(),
            class,
            hardcore,
            expansion,
        };
        match self.account_request(&req, kind::CHAR_CREATED)? {
            AccountEvent::CharCreated(c) => Ok(c),
            _ => Err(OnlineError::Malformed),
        }
    }

    /// Deletes a character; `password` is the account's, to confirm.
    pub fn delete_character(&mut self, name: &str, password: &str) -> Result<(), OnlineError> {
        let req = AccountRequest::CharDelete {
            name: name.into(),
            password: password.into(),
        };
        match self.account_request(&req, kind::CHAR_DELETED)? {
            AccountEvent::CharDeleted { .. } => Ok(()),
            _ => Err(OnlineError::Malformed),
        }
    }

    /// Picks the character the lobby plays with; its summary carries the
    /// unlocked difficulties the lobby decides with.
    pub fn select_character(&mut self, name: &str) -> Result<CharSummary, OnlineError> {
        let req = AccountRequest::CharSelect { name: name.into() };
        match self.account_request(&req, kind::CHAR_SELECTED)? {
            AccountEvent::CharSelected(c) => Ok(c),
            _ => Err(OnlineError::Malformed),
        }
    }
}
