//! The account and character messages of the control stream (kinds
//! 0x20-0x3F, contracts section 3.5): register, log in and out, and the
//! account's online characters (list, create, delete, select).
//!
//! Every request but [`AccountRequest::Register`] and
//! [`AccountRequest::Login`] needs a logged-in session; a failure is
//! answered with [`super::ErrorMsg`]. Online characters live on the server
//! only: a client sees a [`CharSummary`] (what character select and the
//! lobby show), never a character record.
//!
//! The decoder's string limits are wider than the account rules on
//! purpose: a name or password that breaks a rule still decodes, so the
//! server can answer with the rule it breaks (`BadAccountName`,
//! `BadPassword`, `BadCharName`) instead of closing the connection as
//! malformed. The rules themselves are the server's.

use super::{Reader, Writer};
use crate::NetError;

/// The account messages' kinds.
pub mod kind {
    pub const REGISTER: u8 = 0x20;
    pub const LOGIN: u8 = 0x21;
    pub const LOGGED_IN: u8 = 0x22;
    pub const LOGOUT: u8 = 0x23;
    pub const CHAR_LIST: u8 = 0x24;
    pub const CHAR_LIST_ANSWER: u8 = 0x25;
    pub const CHAR_CREATE: u8 = 0x26;
    pub const CHAR_CREATED: u8 = 0x27;
    pub const CHAR_DELETE: u8 = 0x28;
    pub const CHAR_DELETED: u8 = 0x29;
    pub const CHAR_SELECT: u8 = 0x2A;
    pub const CHAR_SELECTED: u8 = 0x2B;
}

/// Longest account name a message carries (the rule allows 15).
pub const MAX_ACCOUNT_WIRE: usize = 32;
/// Longest password a message carries (the rule allows 64 bytes).
pub const MAX_PASSWORD_WIRE: usize = 128;
/// Longest character name a message carries (the rule allows 15).
pub const MAX_CHAR_NAME_WIRE: usize = 32;
/// Longest appearance field of a [`CharSummary`].
pub const MAX_LOOK: usize = 64;
/// Most characters one [`AccountEvent::CharList`] answer carries: the
/// highest `max_characters` a server may be configured with. A full list
/// of the longest summaries still fits one frame.
pub const MAX_CHARACTERS_LIMIT: usize = 32;
/// Character classes: 0 Amazon, 1 Sorceress, 2 Necromancer, 3 Paladin,
/// 4 Barbarian, 5 Druid, 6 Assassin.
pub const CLASSES: u8 = 7;
/// The difficulty mask's bits: 0 Normal, 1 Nightmare, 2 Hell.
pub const DIFFICULTY_MASK: u8 = 0b111;

/// An online character as character select and the lobby see it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharSummary {
    /// The name as stored, with its case.
    pub name: String,
    /// 0-6 (see [`CLASSES`]).
    pub class: u8,
    pub level: u8,
    /// The save status word: hardcore, dead, expansion and progression.
    pub status: u16,
    /// The unlocked difficulties (bit 0 Normal, 1 Nightmare, 2 Hell), from
    /// the status word by the server's difficulty rule.
    pub difficulties: u8,
    /// The save header's appearance bytes, for the character-select picture.
    pub look: Vec<u8>,
    /// Unix seconds of the last game, 0 if never played.
    pub last_played: u64,
}

impl CharSummary {
    pub fn encode(&self, w: &mut Writer) {
        w.str8(&self.name)
            .u8(self.class)
            .u8(self.level)
            .u16(self.status)
            .u8(self.difficulties)
            .bytes8(&self.look)
            .u64(self.last_played);
    }

    pub fn decode(r: &mut Reader) -> Result<Self, NetError> {
        let c = Self {
            name: r.str8(MAX_CHAR_NAME_WIRE)?,
            class: r.u8()?,
            level: r.u8()?,
            status: r.u16()?,
            difficulties: r.u8()?,
            look: r.bytes8(MAX_LOOK)?,
            last_played: r.u64()?,
        };
        if c.name.is_empty() || c.class >= CLASSES || c.difficulties & !DIFFICULTY_MASK != 0 {
            return Err(NetError::Malformed);
        }
        Ok(c)
    }
}

/// C→S account requests.
///
/// Its `Debug` hides passwords, so a request can be logged as it is.
#[derive(Clone, PartialEq, Eq)]
pub enum AccountRequest {
    /// Makes an account and logs it in (answered with
    /// [`AccountEvent::LoggedIn`]).
    Register {
        account: String,
        password: String,
    },
    Login {
        account: String,
        password: String,
    },
    /// Ends the session; the server answers with `BYE`.
    Logout,
    CharList,
    CharCreate {
        name: String,
        class: u8,
        hardcore: bool,
        expansion: bool,
    },
    /// Deletes a character; the account password confirms it.
    CharDelete {
        name: String,
        password: String,
    },
    /// Picks the character the lobby plays with.
    CharSelect {
        name: String,
    },
}

impl std::fmt::Debug for AccountRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const HIDDEN: &str = "<hidden>";
        match self {
            Self::Register { account, .. } => f
                .debug_struct("Register")
                .field("account", account)
                .field("password", &HIDDEN)
                .finish(),
            Self::Login { account, .. } => f
                .debug_struct("Login")
                .field("account", account)
                .field("password", &HIDDEN)
                .finish(),
            Self::Logout => f.write_str("Logout"),
            Self::CharList => f.write_str("CharList"),
            Self::CharCreate {
                name,
                class,
                hardcore,
                expansion,
            } => f
                .debug_struct("CharCreate")
                .field("name", name)
                .field("class", class)
                .field("hardcore", hardcore)
                .field("expansion", expansion)
                .finish(),
            Self::CharDelete { name, .. } => f
                .debug_struct("CharDelete")
                .field("name", name)
                .field("password", &HIDDEN)
                .finish(),
            Self::CharSelect { name } => f.debug_struct("CharSelect").field("name", name).finish(),
        }
    }
}

impl AccountRequest {
    pub fn kind(&self) -> u8 {
        match self {
            Self::Register { .. } => kind::REGISTER,
            Self::Login { .. } => kind::LOGIN,
            Self::Logout => kind::LOGOUT,
            Self::CharList => kind::CHAR_LIST,
            Self::CharCreate { .. } => kind::CHAR_CREATE,
            Self::CharDelete { .. } => kind::CHAR_DELETE,
            Self::CharSelect { .. } => kind::CHAR_SELECT,
        }
    }

    /// Whether the request works without a logged-in session.
    pub fn before_login(&self) -> bool {
        matches!(self, Self::Register { .. } | Self::Login { .. })
    }

    pub fn body(&self) -> Vec<u8> {
        let mut w = Writer::new(self.kind());
        match self {
            Self::Register { account, password } | Self::Login { account, password } => {
                w.str8(account).str8(password);
            }
            Self::Logout | Self::CharList => {}
            Self::CharCreate {
                name,
                class,
                hardcore,
                expansion,
            } => {
                w.str8(name).u8(*class).bool(*hardcore).bool(*expansion);
            }
            Self::CharDelete { name, password } => {
                w.str8(name).str8(password);
            }
            Self::CharSelect { name } => {
                w.str8(name);
            }
        }
        w.finish()
    }

    /// Reads a request body (kind byte included).
    pub fn parse(body: &[u8]) -> Result<Self, NetError> {
        let (k, mut r) = Reader::new(body)?;
        let m = match k {
            kind::REGISTER => Self::Register {
                account: r.str8(MAX_ACCOUNT_WIRE)?,
                password: r.str8(MAX_PASSWORD_WIRE)?,
            },
            kind::LOGIN => Self::Login {
                account: r.str8(MAX_ACCOUNT_WIRE)?,
                password: r.str8(MAX_PASSWORD_WIRE)?,
            },
            kind::LOGOUT => Self::Logout,
            kind::CHAR_LIST => Self::CharList,
            kind::CHAR_CREATE => {
                let name = r.str8(MAX_CHAR_NAME_WIRE)?;
                let class = r.u8()?;
                if class >= CLASSES {
                    return Err(NetError::Malformed);
                }
                Self::CharCreate {
                    name,
                    class,
                    hardcore: r.bool()?,
                    expansion: r.bool()?,
                }
            }
            kind::CHAR_DELETE => Self::CharDelete {
                name: r.str8(MAX_CHAR_NAME_WIRE)?,
                password: r.str8(MAX_PASSWORD_WIRE)?,
            },
            kind::CHAR_SELECT => Self::CharSelect {
                name: r.str8(MAX_CHAR_NAME_WIRE)?,
            },
            _ => return Err(NetError::Malformed),
        };
        r.finish()?;
        Ok(m)
    }
}

/// S→C account answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountEvent {
    /// The session is logged in (after `Register` or `Login`).
    LoggedIn {
        /// The account name as stored, with its case.
        account: String,
        /// How many characters the account may have.
        max_characters: u8,
    },
    /// The answer to `CharList`: the account's characters.
    CharList(Vec<CharSummary>),
    CharCreated(CharSummary),
    CharDeleted {
        name: String,
    },
    /// The lobby plays with this character now.
    CharSelected(CharSummary),
}

impl AccountEvent {
    pub fn kind(&self) -> u8 {
        match self {
            Self::LoggedIn { .. } => kind::LOGGED_IN,
            Self::CharList(_) => kind::CHAR_LIST_ANSWER,
            Self::CharCreated(_) => kind::CHAR_CREATED,
            Self::CharDeleted { .. } => kind::CHAR_DELETED,
            Self::CharSelected(_) => kind::CHAR_SELECTED,
        }
    }

    /// The body. A list longer than [`MAX_CHARACTERS_LIMIT`] is cut there;
    /// the server's `max_characters` never allows one.
    pub fn body(&self) -> Vec<u8> {
        let mut w = Writer::new(self.kind());
        match self {
            Self::LoggedIn {
                account,
                max_characters,
            } => {
                w.str8(account).u8(*max_characters);
            }
            Self::CharList(chars) => {
                let n = chars.len().min(MAX_CHARACTERS_LIMIT);
                w.u8(n as u8);
                for c in &chars[..n] {
                    c.encode(&mut w);
                }
            }
            Self::CharCreated(c) | Self::CharSelected(c) => c.encode(&mut w),
            Self::CharDeleted { name } => {
                w.str8(name);
            }
        }
        w.finish()
    }

    /// Reads an answer body (kind byte included).
    pub fn parse(body: &[u8]) -> Result<Self, NetError> {
        let (k, mut r) = Reader::new(body)?;
        let m = match k {
            kind::LOGGED_IN => Self::LoggedIn {
                account: r.str8(MAX_ACCOUNT_WIRE)?,
                max_characters: r.u8()?,
            },
            kind::CHAR_LIST_ANSWER => {
                let n = r.u8()? as usize;
                if n > MAX_CHARACTERS_LIMIT {
                    return Err(NetError::Malformed);
                }
                let mut chars = Vec::with_capacity(n);
                for _ in 0..n {
                    chars.push(CharSummary::decode(&mut r)?);
                }
                Self::CharList(chars)
            }
            kind::CHAR_CREATED => Self::CharCreated(CharSummary::decode(&mut r)?),
            kind::CHAR_DELETED => Self::CharDeleted {
                name: r.str8(MAX_CHAR_NAME_WIRE)?,
            },
            kind::CHAR_SELECTED => Self::CharSelected(CharSummary::decode(&mut r)?),
            _ => return Err(NetError::Malformed),
        };
        r.finish()?;
        Ok(m)
    }
}

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;
