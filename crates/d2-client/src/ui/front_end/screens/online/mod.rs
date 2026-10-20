//! The online screens: login and account creation, the account's character
//! list, character creation, and the lobby (game list, create game, join
//! game, password prompt, chat). The flow (`flow::next`):
//!
//! ```text
//! main menu "ONLINE" / `play --online` -> login -> characters -> lobby -> game
//!                                          |          |  ^        |
//!                                          |          v  |        +-- create-game panel, password prompt
//!                                          |        create        (pop-ups of the lobby)
//! ```
//!
//! Back always goes one step up: lobby -> characters -> login (logs out) ->
//! main menu. A failed request shows the error's text in the game's message
//! box and the screen stays where it was; a lost connection goes back to
//! login with the reason.
//!
//! All screens share one [`OnlineHandle`]: the connection ([`OnlineLink`])
//! and what the server told so far ([`OnlineState`]). The online character
//! never touches the save folder; the create screen runs with its own sink.

pub mod chars;
pub mod create;
pub mod fake;
pub mod lobby;
pub mod login;
pub mod widgets;

pub mod link;
#[cfg(feature = "online")]
pub mod net_link;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub use link::{
    failure_text, CharInfo, ErrorCode, Event, Failure, GameInfo, Joined, NoLink, OnlineLink,
    Request, ServerEntry,
};

use crate::ui::front_end::{Action, Registry, ScreenId};

pub const ONLINE_LOGIN: ScreenId = ScreenId("online_login");
pub const ONLINE_CHARS: ScreenId = ScreenId("online_chars");
pub const ONLINE_CREATE: ScreenId = ScreenId("online_create");
pub const ONLINE_LOBBY: ScreenId = ScreenId("online_lobby");

/// The chat lines the lobby keeps (no history on the server).
pub const CHAT_KEEP: usize = 64;

/// What the server has told the client so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnlineState {
    /// The servers of `servers.toml` (empty: the file is missing).
    pub servers: Vec<ServerEntry>,
    /// The server the login screen uses.
    pub server: usize,
    /// A handshake is done.
    pub connected: bool,
    pub server_name: String,
    pub registration_open: bool,
    /// Logged in as (the stored spelling).
    pub account: Option<String>,
    pub max_characters: u8,
    /// The account's characters, as the server last listed them.
    pub chars: Vec<CharInfo>,
    /// The character the lobby runs with (`CharSelected`).
    pub selected: Option<CharInfo>,
    /// The open games of the selected character's kind.
    pub games: Vec<GameInfo>,
    /// Lobby chat, oldest first: (from, text).
    pub chat: VecDeque<(String, String)>,
    /// The server answered `Chat` with `NotSupported`: the chat box hides.
    pub chat_off: bool,
    /// The game the server let the character into; the host takes it
    /// ([`Online::take_joined`]) when the front end ends in a game load.
    pub joined: Option<Joined>,
    /// A message the next online screen shows when it opens (the reason a
    /// connection was lost or a game closed).
    pub notice: Option<String>,
}

/// The connection and the state, shared by every online screen.
pub struct Online {
    pub link: Box<dyn OnlineLink>,
    pub state: OnlineState,
}

pub type OnlineHandle = Rc<RefCell<Online>>;

impl Online {
    pub fn new(link: Box<dyn OnlineLink>, servers: Vec<ServerEntry>) -> OnlineHandle {
        Rc::new(RefCell::new(Online {
            link,
            state: OnlineState {
                servers,
                ..OnlineState::default()
            },
        }))
    }

    pub fn send(&mut self, r: Request) {
        self.link.send(r);
    }

    /// Reads every event that arrived, applies the ones that only update the
    /// state (list entries, chat, the character list), and returns them all
    /// in order for the current screen to react to.
    pub fn take_events(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Some(ev) = self.link.poll() {
            self.apply(&ev);
            out.push(ev);
        }
        out
    }

    fn apply(&mut self, ev: &Event) {
        let s = &mut self.state;
        match ev {
            Event::Connected {
                server_name,
                registration_open,
            } => {
                s.connected = true;
                s.server_name = server_name.clone();
                s.registration_open = *registration_open;
            }
            Event::LoggedIn {
                account,
                max_characters,
            } => {
                s.account = Some(account.clone());
                s.max_characters = *max_characters;
            }
            Event::Chars(list) => s.chars = list.clone(),
            Event::CharCreated(c) => {
                s.chars.retain(|o| !o.name.eq_ignore_ascii_case(&c.name));
                s.chars.insert(0, c.clone());
            }
            Event::CharDeleted(name) => s.chars.retain(|o| !o.name.eq_ignore_ascii_case(name)),
            Event::CharSelected(c) => {
                s.selected = Some(c.clone());
                s.games.clear();
            }
            Event::GameListBegin => s.games.clear(),
            Event::Game(g) => match s.games.iter_mut().find(|o| o.id == g.id) {
                Some(o) => *o = g.clone(),
                None => s.games.push(g.clone()),
            },
            Event::GameListEnd => {}
            Event::GameGone(id) => s.games.retain(|g| g.id != *id),
            Event::JoinOk(j) => s.joined = Some(j.clone()),
            Event::ChatLine { from, text } => {
                s.chat.push_back((from.clone(), text.clone()));
                while s.chat.len() > CHAT_KEEP {
                    s.chat.pop_front();
                }
            }
            Event::Error(f) if f.code == link::ErrorCode::NOT_SUPPORTED => s.chat_off = true,
            Event::Error(_) => {}
            Event::Disconnected(f) => {
                let servers = std::mem::take(&mut s.servers);
                let server = s.server;
                *s = OnlineState {
                    servers,
                    server,
                    notice: Some(failure_text(f)),
                    ..OnlineState::default()
                };
            }
        }
    }

    /// The game the lobby joined, once (the host starts the game stream
    /// with its ticket).
    pub fn take_joined(&mut self) -> Option<Joined> {
        self.state.joined.take()
    }

    /// The host lost the connection (during a game, or before the front
    /// end opened): the state starts over and the login screen shows
    /// `text`.
    pub fn lost(&mut self, text: &str) {
        self.apply(&Event::Disconnected(Failure {
            code: ErrorCode(0),
            wait_s: 0,
            text: text.to_string(),
        }));
    }

    /// The login screen starts on the server with id `id` (`play --online
    /// --server ID`); an unknown id keeps the first.
    pub fn select_server(&mut self, id: &str) {
        if let Some(i) = self.state.servers.iter().position(|s| s.id == id) {
            self.state.server = i;
        }
    }
}

/// The UI element name of an online screen's own control (the ids of
/// `screens::element_ids`); `None` for anything else, the select and create
/// screens' controls under the online list and create included.
pub fn control_name(screen: ScreenId, action: Action) -> Option<String> {
    let Action::Custom(a) = action else {
        return None;
    };
    let name = match screen {
        s if s == ONLINE_LOGIN => {
            use login::act::*;
            match a {
                LOGIN => "login".into(),
                NEW_ACCOUNT => "new_account".into(),
                SERVER => "server".into(),
                BACK => "back".into(),
                MESSAGE_OK => "message_ok".into(),
                CANCEL => "cancel".into(),
                f if (FIELD..FIELD + 3).contains(&f) => format!("field({})", f - FIELD),
                _ => return None,
            }
        }
        s if s == ONLINE_CHARS => {
            use chars::act::*;
            match a {
                MESSAGE_OK => "message_ok".into(),
                CANCEL => "cancel".into(),
                DELETE_OK => "delete_ok".into(),
                DELETE_CANCEL => "delete_cancel".into(),
                FIELD => "field(0)".into(),
                _ => return None,
            }
        }
        s if s == ONLINE_CREATE => {
            use create::act::*;
            match a {
                MESSAGE_OK => "message_ok".into(),
                CANCEL => "cancel".into(),
                _ => return None,
            }
        }
        s if s == ONLINE_LOBBY => {
            use lobby::act::*;
            match a {
                BACK => "back".into(),
                PRIMARY => "primary".into(),
                TAB_CREATE => "tab_create".into(),
                TAB_JOIN => "tab_join".into(),
                MESSAGE_OK => "message_ok".into(),
                CANCEL => "cancel".into(),
                PROMPT_OK => "prompt_ok".into(),
                PROMPT_CANCEL => "prompt_cancel".into(),
                SCROLL_UP => "scroll_up".into(),
                SCROLL_DOWN => "scroll_down".into(),
                f if (FIELD..FIELD + 3).contains(&f) => format!("field({})", f - FIELD),
                d if (DIFFICULTY..DIFFICULTY + 3).contains(&d) => format!(
                    "difficulty.{}",
                    link::difficulty_name((d - DIFFICULTY) as u8).to_ascii_lowercase()
                ),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(name)
}

/// Register the online screens on `online`.
pub fn register_with(reg: &mut Registry, online: OnlineHandle) {
    login::register(reg, online.clone());
    chars::register(reg, online.clone());
    create::register(reg, online.clone());
    lobby::register(reg, online);
}

/// Register the online screens with no link: logging in says online play is
/// not in this build. The host replaces them with [`register_with`].
pub fn register(reg: &mut Registry) {
    register_with(reg, Online::new(Box::<NoLink>::default(), Vec::new()));
}
