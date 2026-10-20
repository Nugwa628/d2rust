//! The lobby's control-stream messages (kinds 0x40-0x5F, contracts
//! section 3.6): the game list (a snapshot, then updates while the client
//! is in the lobby), create, join, leaving the lobby, and chat.
//!
//! Every lobby request needs a selected character; a failure is answered
//! with [`super::ErrorMsg`]. The list never carries a password: a game
//! only says whether it has one ([`GameInfo::locked`]), and whether the
//! selected character can join it ([`GameInfo::joinable`], an error code).

use super::{ErrorCode, Reader, Writer};
use crate::NetError;

/// The lobby's kinds.
pub mod kind {
    pub const LIST_GAMES: u8 = 0x40;
    pub const GAME_LIST_BEGIN: u8 = 0x41;
    pub const GAME_ENTRY: u8 = 0x42;
    pub const GAME_LIST_END: u8 = 0x43;
    pub const GAME_GONE: u8 = 0x44;
    pub const CREATE_GAME: u8 = 0x45;
    pub const JOIN_GAME: u8 = 0x46;
    pub const JOIN_OK: u8 = 0x47;
    pub const LEAVE_LOBBY: u8 = 0x48;
    pub const CHAT: u8 = 0x49;
    pub const CHAT_LINE: u8 = 0x4A;
}

/// Game names: 1-15 characters (contracts section 6).
pub const MAX_GAME_NAME: usize = 15;
/// Game passwords: 0-15 bytes; empty = a public game.
pub const MAX_GAME_PASSWORD: usize = 15;
/// A chat line: 1-200 bytes.
pub const MAX_CHAT: usize = 200;
/// A character name as the chat names its sender (the save name rule's
/// longest, 15, with room for a mod game's longer names).
pub const MAX_FROM: usize = 32;
/// Difficulties: 0 Normal, 1 Nightmare, 2 Hell.
pub const DIFFICULTIES: u8 = 3;

/// One open game as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameInfo {
    pub game_id: u32,
    pub name: String,
    /// 0 Normal, 1 Nightmare, 2 Hell.
    pub difficulty: u8,
    pub players: u8,
    pub max_players: u8,
    /// The game has a password.
    pub locked: bool,
    /// The creator's character kind: only characters of the same kind join.
    pub hardcore: bool,
    pub expansion: bool,
    /// 0: the selected character can join; else the code why not
    /// ([`ErrorCode`]), so the list can grey the game out with the reason.
    pub joinable: u16,
}

impl GameInfo {
    fn encode(&self, w: &mut Writer) {
        w.u32(self.game_id)
            .str8(&self.name)
            .u8(self.difficulty)
            .u8(self.players)
            .u8(self.max_players)
            .bool(self.locked)
            .bool(self.hardcore)
            .bool(self.expansion)
            .u16(self.joinable);
    }

    fn decode(r: &mut Reader) -> Result<Self, NetError> {
        let g = Self {
            game_id: r.u32()?,
            name: r.str8(MAX_GAME_NAME)?,
            difficulty: r.u8()?,
            players: r.u8()?,
            max_players: r.u8()?,
            locked: r.bool()?,
            hardcore: r.bool()?,
            expansion: r.bool()?,
            joinable: r.u16()?,
        };
        if g.difficulty >= DIFFICULTIES {
            return Err(NetError::Malformed);
        }
        Ok(g)
    }

    /// Whether the selected character can join (as the server said).
    pub fn can_join(&self) -> bool {
        self.joinable == 0
    }

    /// Why not, as an error code.
    pub fn refusal(&self) -> Option<ErrorCode> {
        (self.joinable != 0).then_some(ErrorCode(self.joinable))
    }
}

/// What a secret field shows in `Debug`: nothing for an empty one, a
/// marker for a set one. A password or ticket never reaches a log.
fn secret(s: &str) -> &'static str {
    if s.is_empty() {
        ""
    } else {
        "<redacted>"
    }
}

/// C→S lobby requests.
///
/// `Debug` never prints a password: it says whether one was set.
#[derive(Clone, PartialEq, Eq)]
pub enum LobbyRequest {
    /// Subscribe to the list: a snapshot, then updates while in the lobby.
    ListGames,
    CreateGame {
        name: String,
        /// Empty: a public game.
        password: String,
        difficulty: u8,
        /// 0: the server's default (its configured `max_players`).
        max_players: u8,
    },
    JoinGame {
        game_id: u32,
        /// Empty for a public game.
        password: String,
    },
    /// Stop the list updates (entering a game, back to character select).
    LeaveLobby,
    Chat {
        text: String,
    },
}

impl std::fmt::Debug for LobbyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ListGames => f.write_str("ListGames"),
            Self::CreateGame {
                name,
                password,
                difficulty,
                max_players,
            } => f
                .debug_struct("CreateGame")
                .field("name", name)
                .field("password", &secret(password))
                .field("difficulty", difficulty)
                .field("max_players", max_players)
                .finish(),
            Self::JoinGame { game_id, password } => f
                .debug_struct("JoinGame")
                .field("game_id", game_id)
                .field("password", &secret(password))
                .finish(),
            Self::LeaveLobby => f.write_str("LeaveLobby"),
            Self::Chat { text } => f.debug_struct("Chat").field("text", text).finish(),
        }
    }
}

impl LobbyRequest {
    pub fn kind(&self) -> u8 {
        match self {
            Self::ListGames => kind::LIST_GAMES,
            Self::CreateGame { .. } => kind::CREATE_GAME,
            Self::JoinGame { .. } => kind::JOIN_GAME,
            Self::LeaveLobby => kind::LEAVE_LOBBY,
            Self::Chat { .. } => kind::CHAT,
        }
    }

    pub fn body(&self) -> Vec<u8> {
        let mut w = Writer::new(self.kind());
        match self {
            Self::ListGames | Self::LeaveLobby => {}
            Self::CreateGame {
                name,
                password,
                difficulty,
                max_players,
            } => {
                w.str8(name).str8(password).u8(*difficulty).u8(*max_players);
            }
            Self::JoinGame { game_id, password } => {
                w.u32(*game_id).str8(password);
            }
            Self::Chat { text } => {
                w.str8(text);
            }
        }
        w.finish()
    }

    /// Reads a request body (kind byte included). Field limits are the
    /// codec's (a decoder refuses over-long strings); the lobby's rules
    /// (name characters, unlocked difficulty) are the server's to answer.
    pub fn parse(body: &[u8]) -> Result<Self, NetError> {
        let (k, mut r) = Reader::new(body)?;
        let m = match k {
            kind::LIST_GAMES => Self::ListGames,
            kind::CREATE_GAME => Self::CreateGame {
                name: r.str8(MAX_GAME_NAME)?,
                password: r.str8(MAX_GAME_PASSWORD)?,
                difficulty: r.u8()?,
                max_players: r.u8()?,
            },
            kind::JOIN_GAME => Self::JoinGame {
                game_id: r.u32()?,
                password: r.str8(MAX_GAME_PASSWORD)?,
            },
            kind::LEAVE_LOBBY => Self::LeaveLobby,
            kind::CHAT => {
                let text = r.str8(MAX_CHAT)?;
                if text.is_empty() {
                    return Err(NetError::Malformed);
                }
                Self::Chat { text }
            }
            _ => return Err(NetError::Malformed),
        };
        r.finish()?;
        Ok(m)
    }
}

/// S→C lobby messages.
///
/// `Debug` never prints a join ticket: it is valid once, and a log is not
/// the place for it.
#[derive(Clone, PartialEq, Eq)]
pub enum LobbyEvent {
    /// A snapshot of `count` games follows (as [`LobbyEvent::GameEntry`]s),
    /// then [`LobbyEvent::GameListEnd`].
    GameListBegin {
        count: u16,
    },
    /// A game of the snapshot, or an update for a new or changed game.
    GameEntry(GameInfo),
    GameListEnd,
    GameGone {
        game_id: u32,
    },
    /// The seat is held: the client opens the game stream with `ticket`.
    JoinOk {
        game_id: u32,
        /// A secret: valid once, for 30 s, on this connection only.
        ticket: [u8; 16],
        difficulty: u8,
    },
    ChatLine {
        from: String,
        text: String,
        /// Unix seconds.
        at: u64,
    },
}

impl std::fmt::Debug for LobbyEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GameListBegin { count } => f
                .debug_struct("GameListBegin")
                .field("count", count)
                .finish(),
            Self::GameEntry(g) => f.debug_tuple("GameEntry").field(g).finish(),
            Self::GameListEnd => f.write_str("GameListEnd"),
            Self::GameGone { game_id } => f
                .debug_struct("GameGone")
                .field("game_id", game_id)
                .finish(),
            Self::JoinOk {
                game_id,
                ticket: _,
                difficulty,
            } => f
                .debug_struct("JoinOk")
                .field("game_id", game_id)
                .field("ticket", &"<redacted>")
                .field("difficulty", difficulty)
                .finish(),
            Self::ChatLine { from, text, at } => f
                .debug_struct("ChatLine")
                .field("from", from)
                .field("text", text)
                .field("at", at)
                .finish(),
        }
    }
}

impl LobbyEvent {
    pub fn kind(&self) -> u8 {
        match self {
            Self::GameListBegin { .. } => kind::GAME_LIST_BEGIN,
            Self::GameEntry(_) => kind::GAME_ENTRY,
            Self::GameListEnd => kind::GAME_LIST_END,
            Self::GameGone { .. } => kind::GAME_GONE,
            Self::JoinOk { .. } => kind::JOIN_OK,
            Self::ChatLine { .. } => kind::CHAT_LINE,
        }
    }

    pub fn body(&self) -> Vec<u8> {
        let mut w = Writer::new(self.kind());
        match self {
            Self::GameListBegin { count } => {
                w.u16(*count);
            }
            Self::GameEntry(g) => g.encode(&mut w),
            Self::GameListEnd => {}
            Self::GameGone { game_id } => {
                w.u32(*game_id);
            }
            Self::JoinOk {
                game_id,
                ticket,
                difficulty,
            } => {
                w.u32(*game_id).array(ticket).u8(*difficulty);
            }
            Self::ChatLine { from, text, at } => {
                w.str8(from).str8(text).u64(*at);
            }
        }
        w.finish()
    }

    pub fn parse(body: &[u8]) -> Result<Self, NetError> {
        let (k, mut r) = Reader::new(body)?;
        let m = match k {
            kind::GAME_LIST_BEGIN => Self::GameListBegin { count: r.u16()? },
            kind::GAME_ENTRY => Self::GameEntry(GameInfo::decode(&mut r)?),
            kind::GAME_LIST_END => Self::GameListEnd,
            kind::GAME_GONE => Self::GameGone { game_id: r.u32()? },
            kind::JOIN_OK => {
                let game_id = r.u32()?;
                let ticket = r.array()?;
                let difficulty = r.u8()?;
                if difficulty >= DIFFICULTIES {
                    return Err(NetError::Malformed);
                }
                Self::JoinOk {
                    game_id,
                    ticket,
                    difficulty,
                }
            }
            kind::CHAT_LINE => Self::ChatLine {
                from: r.str8(MAX_FROM)?,
                text: r.str8(MAX_CHAT)?,
                at: r.u64()?,
            },
            _ => return Err(NetError::Malformed),
        };
        r.finish()?;
        Ok(m)
    }
}

#[cfg(test)]
#[path = "lobby_tests.rs"]
mod tests;
