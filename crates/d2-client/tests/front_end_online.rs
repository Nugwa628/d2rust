//! The online screens on the in-memory fake server, no game files: the
//! flow main menu -> login -> characters -> lobby -> game, and the rules
//! the screens show (wrong login, account rules, password games, the
//! difficulty rule from the character's progression, a lost connection).

use d2_client::ui::front_end::screens::online::fake::{character, FakeServer, SharedFake};
use d2_client::ui::front_end::screens::online::lobby::{Lobby, View};
use d2_client::ui::front_end::screens::online::{self, *};
use d2_client::ui::front_end::*;
use d2_formats::d2s::status;

const ESC: u16 = 27;
const ENTER: u16 = 13;
const TAB: u16 = 9;

struct Rig {
    f: FrontEnd,
    server: SharedFake,
    online: OnlineHandle,
}

fn servers() -> Vec<ServerEntry> {
    vec![ServerEntry {
        id: "eu1".into(),
        name: "D2RUST Test".into(),
        address: "127.0.0.1:6112".into(),
    }]
}

fn rig_with(server: FakeServer, servers: Vec<ServerEntry>) -> Rig {
    let server = SharedFake(std::rc::Rc::new(std::cell::RefCell::new(server)));
    let online = Online::new(Box::new(server.clone()), servers);
    let mut reg = Registry::default();
    screens::register_all(&mut reg);
    online::register_with(&mut reg, online.clone());
    let mut f = FrontEnd::new(true, Box::new(false), reg);
    f.start(false);
    Rig { f, server, online }
}

/// A server with one account, `mogger` / `password1`, holding a level 30
/// expansion barbarian that unlocked Nightmare (progression 5) and a fresh
/// sorceress.
fn rig() -> Rig {
    let mut s = FakeServer::default();
    s.add_account(
        "Mogger",
        "password1",
        vec![
            character("Bar", 4, 30, 5, status::EXPANSION),
            character("Sorc", 1, 1, 0, status::EXPANSION),
        ],
    );
    rig_with(s, servers())
}

impl Rig {
    fn ticks(&mut self, n: u32) {
        for _ in 0..n {
            self.f.tick();
        }
    }

    fn ids(&self) -> Vec<String> {
        self.f.ui_elements().into_iter().map(|e| e.id).collect()
    }

    fn click(&mut self, id: &str) {
        let e = self
            .f
            .ui_elements()
            .into_iter()
            .find(|e| e.id == id)
            .unwrap_or_else(|| panic!("no {id} on {:?}: {:?}", self.f.current(), self.ids()));
        self.f.input(FrontInput::Down(e.at));
        self.f.input(FrontInput::Up(e.at));
        self.ticks(2);
    }

    fn key(&mut self, vk: u16) {
        self.f.input(FrontInput::Key(vk));
        self.ticks(2);
    }

    fn typed(&mut self, s: &str) {
        for u in s.encode_utf16() {
            self.f.input(FrontInput::Char(u));
        }
        self.ticks(1);
    }

    /// Every text the screen shows now (controls and overlay).
    fn texts(&mut self) -> Vec<String> {
        let mut items = self.f.draw();
        items.extend(self.f.overlay(&|_, t| t.len() as i32 * 8));
        items
            .into_iter()
            .filter_map(|i| match i {
                DrawItem::Text { text, .. } if !text.is_empty() => Some(text),
                _ => None,
            })
            .collect()
    }

    fn shows(&mut self, s: &str) -> bool {
        self.texts().iter().any(|t| t.contains(s))
    }

    /// Main menu -> login (the flow's trigger: the button is there only
    /// with the `online` feature).
    fn open_login(&mut self) {
        self.f.trigger(Trigger::Online);
        self.ticks(1);
    }

    fn login(&mut self, account: &str, password: &str) {
        self.open_login();
        assert_eq!(self.f.current(), ONLINE_LOGIN);
        self.typed(account);
        self.key(TAB);
        self.typed(password);
        self.key(ENTER);
        self.ticks(2);
    }

    fn enter_lobby(&mut self, name: &str) {
        self.login("mogger", "password1");
        assert_eq!(self.f.current(), ONLINE_CHARS);
        self.click(&format!("online_chars.char({name})"));
        self.click("online_chars.ok");
        self.ticks(2);
        assert_eq!(self.f.current(), ONLINE_LOBBY, "{:?}", self.texts());
    }

    fn seen(&self) -> Vec<Request> {
        self.server.0.borrow().seen.clone()
    }
}

/// Rule (contracts 9): with the `online` feature the main menu's ONLINE
/// opens the login screen; without it there is no ONLINE button.
#[test]
fn main_menu_online_button_follows_the_feature() {
    let mut r = rig();
    let has = r.ids().contains(&"menu.online".to_string());
    assert_eq!(has, cfg!(feature = "online"));
    assert_eq!(r.shows("ONLINE"), cfg!(feature = "online"));
    if has {
        r.click("menu.online");
        assert_eq!(r.f.current(), ONLINE_LOGIN);
    }
}

/// Rule (contracts 9): the login screen names the server; BACK returns to
/// the main menu.
#[test]
fn login_names_the_server_and_back_returns() {
    let mut r = rig();
    r.open_login();
    assert_eq!(r.f.current(), ONLINE_LOGIN);
    assert!(r.shows("SERVER: D2RUST TEST"));
    r.key(ESC);
    assert_eq!(r.f.current(), MAIN_MENU);
}

/// Rule (contracts 3.4, 3.5): LOG IN connects, then logs in; the list
/// screen asks for the account's characters and shows them.
#[test]
fn login_connects_then_lists_characters() {
    let mut r = rig();
    r.login("mogger", "password1");
    assert_eq!(r.f.current(), ONLINE_CHARS);
    let seen = r.seen();
    assert_eq!(seen[0], Request::Connect { index: 0 });
    assert_eq!(
        seen[1],
        Request::Login {
            account: "mogger".into(),
            password: "password1".into()
        }
    );
    assert_eq!(seen[2], Request::CharList);
    let ids = r.ids();
    assert!(
        ids.contains(&"online_chars.char(Bar)".to_string()),
        "{ids:?}"
    );
    assert!(ids.contains(&"online_chars.char(Sorc)".to_string()));
    assert!(r.shows("Mogger"));
}

/// Rule (contracts 3.8 code 23): a wrong password shows one message for
/// unknown account and wrong password; the screen stays.
#[test]
fn wrong_login_says_so_and_stays() {
    let mut r = rig();
    r.login("mogger", "nope-nope");
    assert_eq!(r.f.current(), ONLINE_LOGIN);
    assert!(r.shows("Wrong account name or password."));
    r.click("online_login.message_ok");
    assert!(!r.shows("Wrong account name"));
    // The typed name stays for the next try.
    assert!(r.shows("mogger"));
}

/// Rule (contracts 4): account names and passwords are checked before the
/// request; a new account logs in at once.
#[test]
fn new_account_rules_and_register() {
    let mut r = rig();
    r.open_login();
    r.click("online_login.new_account");
    assert!(r.shows("NEW ACCOUNT"));
    r.typed("x");
    r.key(TAB);
    r.typed("password1");
    r.key(TAB);
    r.typed("password1");
    r.key(ENTER);
    assert!(r.shows("Account names are 2-15"));
    assert!(r.seen().is_empty(), "nothing sent for a bad name");
    r.click("online_login.message_ok");
    r.click("online_login.field(0)");
    r.typed("yz");
    r.click("online_login.field(2)");
    r.typed("!");
    r.key(ENTER);
    assert!(r.shows("The two passwords are not the same."));
    r.click("online_login.message_ok");
    r.click("online_login.field(2)");
    r.f.input(FrontInput::Key(8));
    r.ticks(1);
    r.key(ENTER);
    assert_eq!(r.f.current(), ONLINE_CHARS);
    assert!(r.seen().contains(&Request::Register {
        account: "xyz".into(),
        password: "password1".into()
    }));
}

/// Rule (contracts 9): without `servers.toml` the login screen says where
/// the list comes from.
#[test]
fn no_server_list_says_to_use_the_launcher() {
    let mut r = rig_with(FakeServer::default(), Vec::new());
    r.open_login();
    assert!(r.shows("Start the game once from the launcher"));
}

/// Rule (owner decision 4, contracts 6): the lobby's list greys out games
/// the character can't join and says why; a password game asks for the
/// password; blank and wrong passwords are refused; the right one starts
/// an online game load.
#[test]
fn lobby_join_rules_and_password_prompt() {
    let mut r = rig();
    {
        let mut s = r.server.0.borrow_mut();
        let game = |name: &str, difficulty: u8, locked: bool| online::GameInfo {
            id: 0,
            name: name.into(),
            difficulty,
            players: 1,
            max_players: 8,
            locked,
            hardcore: false,
            expansion: true,
            joinable: 0,
        };
        s.add_game(game("cows", 0, false), "");
        s.add_game(game("baal", 2, false), "");
        s.add_game(game("secret", 1, true), "moo");
    }
    r.enter_lobby("Bar");
    assert!(r.shows("cows"));
    assert!(r.shows("Difficulties: Normal, Nightmare"));
    // Hell is locked for this character: the list says why.
    r.click("online_lobby.game(baal)");
    assert!(r.shows("Hell is not unlocked yet."));
    r.click("online_lobby.primary");
    assert!(r.shows("Your character has not unlocked that difficulty."));
    r.click("online_lobby.message_ok");
    // The password game: prompt, blank refused, wrong refused, right joins.
    r.click("online_lobby.game(secret)");
    r.click("online_lobby.primary");
    assert!(r.shows("This game needs a password."));
    r.click("online_lobby.prompt_ok");
    assert!(r.shows("That game needs a password."));
    r.click("online_lobby.message_ok");
    r.click("online_lobby.primary");
    r.typed("oink");
    r.click("online_lobby.prompt_ok");
    r.ticks(2);
    assert!(r.shows("Wrong game password."), "{:?}", r.texts());
    r.click("online_lobby.message_ok");
    r.click("online_lobby.primary");
    r.typed("moo");
    r.key(ENTER);
    r.ticks(2);
    assert_eq!(
        r.f.outcome(),
        Some(Outcome::GameLoad(GameLoad {
            difficulty: None,
            new_character: false,
            online: true,
        }))
    );
    let j = r.online.borrow_mut().take_joined().expect("joined");
    assert_eq!((j.difficulty, j.name.as_str()), (1, "secret"));
    assert!(r.seen().contains(&Request::LeaveLobby));
}

/// Rule (contracts 6): a public game (blank password) joins with no prompt.
#[test]
fn public_game_joins_at_once() {
    let mut r = rig();
    r.server.0.borrow_mut().add_game(
        online::GameInfo {
            id: 0,
            name: "open".into(),
            difficulty: 0,
            players: 2,
            max_players: 8,
            locked: false,
            hardcore: false,
            expansion: true,
            joinable: 0,
        },
        "",
    );
    r.enter_lobby("Sorc");
    r.click("online_lobby.game(open)");
    r.click("online_lobby.primary");
    r.ticks(2);
    assert!(matches!(r.f.outcome(), Some(Outcome::GameLoad(g)) if g.online));
    assert!(r.seen().contains(&Request::JoinGame {
        game_id: 1,
        password: String::new()
    }));
}

/// Rule (owner decision 4): the create form offers only the difficulties
/// the character unlocked; a fresh character creates at Normal only.
#[test]
fn create_form_limits_difficulty_to_progression() {
    let mut r = rig();
    r.enter_lobby("Sorc");
    r.click("online_lobby.tab_create");
    let ids = r.ids();
    assert!(ids.contains(&"online_lobby.difficulty.normal".to_string()));
    assert!(!ids.contains(&"online_lobby.difficulty.nightmare".to_string()));
    assert!(!ids.contains(&"online_lobby.difficulty.hell".to_string()));
    assert!(r.shows("Grey: not unlocked"));
    r.click("online_lobby.field(0)");
    r.typed("my game");
    r.click("online_lobby.primary");
    r.ticks(2);
    assert!(matches!(r.f.outcome(), Some(Outcome::GameLoad(g)) if g.online));
    assert!(r.seen().contains(&Request::CreateGame {
        name: "my game".into(),
        password: String::new(),
        difficulty: 0
    }));
}

/// Rule (owner decision 4): a character with Nightmare unlocked starts the
/// form at its highest difficulty and may pick Normal.
#[test]
fn create_form_defaults_to_highest_unlocked() {
    let mut r = rig();
    r.enter_lobby("Bar");
    r.click("online_lobby.tab_create");
    let ids = r.ids();
    assert!(ids.contains(&"online_lobby.difficulty.nightmare".to_string()));
    assert!(!ids.contains(&"online_lobby.difficulty.hell".to_string()));
    r.click("online_lobby.field(0)");
    r.typed("nm run");
    r.click("online_lobby.field(1)");
    r.typed("pw");
    r.click("online_lobby.primary");
    r.ticks(2);
    assert!(r.seen().contains(&Request::CreateGame {
        name: "nm run".into(),
        password: "pw".into(),
        difficulty: 1
    }));
}

/// Rule (contracts 3.6): the list follows the server's updates while the
/// player is in the lobby; chat lines show; back out goes to the list.
#[test]
fn lobby_follows_updates_and_chat() {
    let mut r = rig();
    r.enter_lobby("Bar");
    assert!(r.shows("No open games"));
    let id = r.server.0.borrow_mut().add_game(
        online::GameInfo {
            id: 0,
            name: "late".into(),
            difficulty: 0,
            players: 1,
            max_players: 8,
            locked: false,
            hardcore: false,
            expansion: true,
            joinable: 0,
        },
        "",
    );
    // The fake sends no update for add_game: the list is asked again.
    r.click("online_lobby.back");
    r.click("online_chars.ok");
    r.ticks(2);
    assert!(r.shows("late"));
    r.server.0.borrow_mut().close_game(id);
    r.ticks(2);
    assert!(!r.shows("late"));
    r.server.0.borrow_mut().chat_from("Friend", "hello there");
    r.ticks(2);
    assert!(r.shows("<Friend> hello there"));
    r.typed("hi");
    r.key(ENTER);
    r.ticks(2);
    assert!(r.shows("<Bar> hi"));
    r.key(ESC);
    assert_eq!(r.f.current(), ONLINE_CHARS);
}

/// Rule (contracts 3.6): a server without chat hides the input line.
#[test]
fn chat_off_hides_input() {
    let mut s = FakeServer::default();
    s.chat_off = true;
    s.add_account(
        "Mogger",
        "password1",
        vec![character("Bar", 4, 30, 5, status::EXPANSION)],
    );
    let mut r = rig_with(s, servers());
    r.enter_lobby("Bar");
    r.typed("hi");
    r.key(ENTER);
    r.ticks(2);
    assert!(r.shows("Chat is not available on this server."));
}

/// Rule (contracts 9): a lost connection goes back to login with the reason.
#[test]
fn lost_connection_returns_to_login_with_reason() {
    let mut r = rig();
    r.enter_lobby("Bar");
    r.server
        .0
        .borrow_mut()
        .drop_connection("The server closed the connection.");
    r.ticks(2);
    assert_eq!(r.f.current(), ONLINE_LOGIN);
    assert!(r.shows("The server closed the connection."));
    assert!(!r.online.borrow().state.connected);
}

/// Rule (contracts 3.5, 9): online create sends the new character to the
/// server (nothing on disk); a taken name keeps the create screen with the
/// reason; created goes back to the list with it.
#[test]
fn online_create_sends_to_server() {
    let mut s = FakeServer::default();
    s.add_account("Mogger", "password1", Vec::new());
    s.add_account("Other", "password2", vec![character("Taken", 0, 1, 0, 0)]);
    let mut r = rig_with(s, servers());
    r.login("mogger", "password1");
    r.click("online_chars.create_new");
    assert_eq!(r.f.current(), ONLINE_CREATE);
    r.click("online_create.class.barbarian");
    r.ticks(30);
    r.typed("Taken");
    r.key(ENTER);
    r.ticks(2);
    assert_eq!(r.f.current(), ONLINE_CREATE);
    assert!(
        r.shows("That character name is already taken."),
        "{:?}",
        r.texts()
    );
    r.click("online_create.message_ok");
    for _ in 0..5 {
        r.f.input(FrontInput::Key(8));
    }
    r.ticks(1);
    r.typed("Fresh");
    r.key(ENTER);
    r.ticks(2);
    assert_eq!(r.f.current(), ONLINE_CHARS);
    assert!(r.ids().contains(&"online_chars.char(Fresh)".to_string()));
    assert!(r
        .seen()
        .iter()
        .any(|q| matches!(q, Request::CharCreate { name, class: 4, .. } if name == "Fresh")));
}

/// Rule (contracts 3.5): delete asks for the account password; the server
/// deletes and the list drops the character.
#[test]
fn delete_asks_for_the_account_password() {
    let mut r = rig();
    r.login("mogger", "password1");
    r.click("online_chars.char(Sorc)");
    r.click("online_chars.delete");
    r.click("online_chars.yes");
    assert!(r.shows("Type your account password to delete Sorc."));
    r.typed("password1");
    r.click("online_chars.delete_ok");
    r.ticks(2);
    assert!(!r.ids().contains(&"online_chars.char(Sorc)".to_string()));
    assert!(r.seen().contains(&Request::CharDelete {
        name: "Sorc".into(),
        password: "password1".into()
    }));
}

/// The lobby's state starts on the list.
#[test]
fn lobby_view_starts_on_list() {
    let o = Online::new(Box::<NoLink>::default(), Vec::new());
    let l = Lobby::new(o);
    assert_eq!(l.view, View::List);
}

// ---- The host's side: the lobby's join becomes the online game, and the
// front end comes back after it (cl-mp-client).

/// A front end on `online` opened at `screen`, the way the host opens it
/// again after a game.
fn reopen(online: &OnlineHandle, screen: ScreenId) -> FrontEnd {
    let mut reg = Registry::default();
    screens::register_all(&mut reg);
    online::register_with(&mut reg, online.clone());
    let mut f = FrontEnd::new(true, Box::new(false), reg);
    f.start(false);
    f.goto(screen);
    f
}

/// A public Normal game the fresh sorceress can join.
fn open_game(r: &Rig) {
    r.server.0.borrow_mut().add_game(
        online::GameInfo {
            id: 0,
            name: "open".into(),
            difficulty: 0,
            players: 2,
            max_players: 8,
            locked: false,
            hardcore: false,
            expansion: true,
            joinable: 0,
        },
        "",
    );
}

fn join_open_game(r: &mut Rig) {
    open_game(r);
    r.enter_lobby("Sorc");
    r.click("online_lobby.game(open)");
    r.click("online_lobby.primary");
    r.ticks(2);
    assert!(matches!(r.f.outcome(), Some(Outcome::GameLoad(g)) if g.online));
}

/// Rule (contracts 9): leaving the game returns to the lobby on the same
/// session; why the game ended shows there, and the lobby lists the games
/// again. A join the game did not use is spent.
#[test]
fn after_a_game_the_lobby_opens_on_the_same_session() {
    use d2_client::app::front_start::{after_online_game_on, Entry};
    let mut r = rig();
    join_open_game(&mut r);
    let lists = |r: &Rig| {
        r.seen()
            .iter()
            .filter(|q| **q == Request::ListGames)
            .count()
    };
    let before = lists(&r);
    assert_eq!(
        after_online_game_on(&r.online, true, Some("The game closed.".into())),
        Entry::Lobby
    );
    assert!(r.online.borrow().state.joined.is_none());
    r.f = reopen(&r.online, ONLINE_LOBBY);
    r.ticks(2);
    assert_eq!(r.f.current(), ONLINE_LOBBY);
    assert!(r.shows("The game closed."));
    assert_eq!(lists(&r), before + 1, "the lobby lists the games again");
    assert_eq!(
        r.online
            .borrow()
            .state
            .selected
            .as_ref()
            .map(|c| c.name.as_str()),
        Some("Sorc")
    );
}

/// Rule (contracts 9): a game whose connection is gone goes back to the
/// login with the reason; the online state starts over.
#[test]
fn after_a_lost_game_the_login_opens_with_the_reason() {
    use d2_client::app::front_start::{after_online_game_on, Entry};
    let mut r = rig();
    join_open_game(&mut r);
    let why = "The connection to the server was lost (timed out).";
    assert_eq!(
        after_online_game_on(&r.online, false, Some(why.into())),
        Entry::OnlineError
    );
    r.f = reopen(&r.online, ONLINE_LOGIN);
    r.ticks(2);
    assert_eq!(r.f.current(), ONLINE_LOGIN);
    assert!(r.shows(why));
    let st = &r.online.borrow().state;
    assert!(!st.connected && st.selected.is_none() && st.joined.is_none());
}

#[cfg(feature = "online")]
mod host {
    use super::*;
    use d2_client::app::front_start::{front_host_with, Entry, StartHandles};
    use d2_client::app::online::OnlineJoin;

    fn two_servers() -> Vec<ServerEntry> {
        let mut s = servers();
        s.push(ServerEntry {
            id: "us1".into(),
            name: "D2RUST West".into(),
            address: "127.0.0.2:6112".into(),
        });
        s
    }

    fn save_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("d2-fe-online-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The host's front end on `handles`, opened at `entry`, as a rig.
    fn host_rig(handles: &StartHandles, server: SharedFake, entry: Entry) -> Rig {
        let dir = save_dir("host");
        let mut host = front_host_with(&dir, None, true, entry, handles);
        let f = std::mem::replace(
            &mut host.front,
            FrontEnd::new(true, Box::new(false), Registry::default()),
        );
        let online = handles.online.borrow().clone().unwrap();
        Rig { f, server, online }
    }

    fn fake_handles(server: &SharedFake, servers: Vec<ServerEntry>) -> StartHandles {
        let h = StartHandles::default();
        *h.online.borrow_mut() = Some(Online::new(Box::new(server.clone()), servers));
        h
    }

    /// Rule (contracts 9): `play --online --server ID` opens the login
    /// screen on that server, past the main menu.
    #[test]
    fn play_online_opens_the_login_on_the_chosen_server() {
        let server = SharedFake(std::rc::Rc::new(std::cell::RefCell::new(
            FakeServer::default(),
        )));
        let mut h = fake_handles(&server, two_servers());
        h.server = Some("us1".into());
        let mut r = host_rig(&h, server, Entry::Online);
        r.ticks(1);
        assert_eq!(r.f.current(), ONLINE_LOGIN);
        assert_eq!(r.online.borrow().state.server, 1);
        assert!(r.shows("SERVER: D2RUST WEST"), "{:?}", r.texts());
    }

    /// Rule (contracts 9, 3.7): the lobby's join becomes the game's join:
    /// the `JoinOk` ticket and difficulty with the selected character's
    /// name, class and status, taken once. The front end opened again on
    /// the same handles comes back to the lobby.
    #[test]
    fn the_lobby_join_is_the_online_games_join() {
        let base = rig();
        let server = base.server.clone();
        let h = fake_handles(&server, servers());
        let mut r = host_rig(&h, server, Entry::MainMenu);
        assert_eq!(r.f.current(), MAIN_MENU);
        join_open_game(&mut r);
        let join = OnlineJoin::take(&mut r.online.borrow_mut()).expect("a join");
        let sorc = r.online.borrow().state.selected.clone().unwrap();
        let joined_ticket = join.ticket;
        assert_eq!(
            join,
            OnlineJoin {
                ticket: joined_ticket,
                name: "Sorc".into(),
                class: 1,
                status: sorc.status,
                difficulty: 0,
            }
        );
        assert!(OnlineJoin::take(&mut r.online.borrow_mut()).is_none());

        let again = host_rig(&h, r.server.clone(), Entry::Lobby);
        assert_eq!(again.f.current(), ONLINE_LOBBY);
        assert!(std::rc::Rc::ptr_eq(&again.online, &r.online));
    }
}

/// The screens and the game start on a real session over loopback QUIC:
/// a server scripted to the contract (accounts, lobby, game attach) behind
/// a pinned certificate, the game's own `NetLink` and `JoinTask`.
#[cfg(feature = "online")]
mod loopback {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use d2_client::app::front_start::{after_online_game, front_host_with, Entry, StartHandles};
    use d2_client::app::online::{self as app_online, JoinTask, OnlineJoin, ServerList};
    use d2_client::ui::front_end::screens::online::net_link::{entries, NetLink};
    use d2_net::msg::account::{AccountEvent, AccountRequest, CharSummary};
    use d2_net::msg::lobby::{GameInfo as WireGame, LobbyEvent, LobbyRequest};
    use d2_net::wire::{Attach, ResumeToken, SessionWelcome, Welcome};
    use d2_net::Timing;
    use d2_online::ServerTrust;
    use d2_quic::{fingerprint, FramedStream, Identity, QuicConn, QuicServer};

    const TICKET: [u8; 16] = [7; 16];
    const GAME_TOKEN: ResumeToken = ResumeToken([9; 16]);
    /// The game server answers the attach this late: the join must not
    /// wait for it on the game thread.
    const ATTACH_DELAY: Duration = Duration::from_millis(300);

    fn sorc() -> CharSummary {
        CharSummary {
            name: "Sorc".into(),
            class: 1,
            level: 12,
            status: status::EXPANSION,
            difficulties: 0b001,
            look: vec![0; 32],
            last_played: 0,
        }
    }

    fn game() -> WireGame {
        WireGame {
            game_id: 3,
            name: "loop".into(),
            difficulty: 0,
            players: 1,
            max_players: 8,
            locked: false,
            hardcore: false,
            expansion: true,
            joinable: 0,
        }
    }

    /// The control stream: handshake, accounts and lobby as the contract
    /// answers them; counts the `ListGames`.
    async fn serve(conn: QuicConn, mut control: FramedStream, lists: Arc<AtomicU32>) {
        if control.recv_async().await.is_err() {
            return;
        }
        let welcome = SessionWelcome {
            tick_rate: d2_sim::tick::TickRate::GAME.hz(),
            max_players: 8,
            session: ResumeToken([5; 16]),
            server_name: "Loopback realm".into(),
            registration_open: false,
            resume: None,
        };
        if control.send(welcome.body()).is_err() {
            return;
        }
        tokio::spawn(serve_games(conn));
        while let Ok(b) = control.recv_async().await {
            let answers: Vec<Vec<u8>> = if let Ok(r) = AccountRequest::parse(&b) {
                match r {
                    AccountRequest::Login { account, .. } => vec![AccountEvent::LoggedIn {
                        account,
                        max_characters: 8,
                    }
                    .body()],
                    AccountRequest::CharList => vec![AccountEvent::CharList(vec![sorc()]).body()],
                    AccountRequest::CharSelect { .. } => {
                        vec![AccountEvent::CharSelected(sorc()).body()]
                    }
                    _ => vec![],
                }
            } else if let Ok(r) = LobbyRequest::parse(&b) {
                match r {
                    LobbyRequest::ListGames => {
                        lists.fetch_add(1, Ordering::SeqCst);
                        vec![
                            LobbyEvent::GameListBegin { count: 1 }.body(),
                            LobbyEvent::GameEntry(game()).body(),
                            LobbyEvent::GameListEnd.body(),
                        ]
                    }
                    LobbyRequest::JoinGame { game_id: 3, .. } => vec![LobbyEvent::JoinOk {
                        game_id: 3,
                        ticket: TICKET,
                        difficulty: 0,
                    }
                    .body()],
                    _ => vec![],
                }
            } else {
                vec![]
            };
            for a in answers {
                let _ = control.send(a);
            }
        }
    }

    /// The game streams: the lobby ticket attaches, late.
    async fn serve_games(conn: QuicConn) {
        while let Some(mut stream) = conn.accept_stream().await {
            let Ok(first) = stream.recv_async().await else {
                continue;
            };
            if let Ok(Attach::Ticket(TICKET)) = Attach::parse(&first) {
                tokio::time::sleep(ATTACH_DELAY).await;
                let w = Welcome {
                    client: 1,
                    tick_rate: d2_sim::tick::TickRate::GAME.hz(),
                    max_players: 8,
                    resume: GAME_TOKEN,
                };
                let _ = stream.send(w.body());
                while stream.recv_async().await.is_ok() {}
            }
        }
    }

    /// Ticks the front end in real time until `done` (at most 10 s).
    fn until(r: &mut Rig, what: &str, mut done: impl FnMut(&mut Rig) -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !done(r) {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
            r.ticks(1);
        }
    }

    #[test]
    fn a_lobby_join_starts_the_game_without_waiting_and_comes_back_to_the_lobby() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let id = Identity::self_signed().unwrap();
        let cert = id.cert.clone();
        let quic = Arc::new(
            QuicServer::bind(
                "127.0.0.1:0".parse().unwrap(),
                id,
                Timing::default(),
                rt.handle().clone(),
            )
            .unwrap(),
        );
        let lists = Arc::new(AtomicU32::new(0));
        {
            let (q, lists) = (quic.clone(), lists.clone());
            rt.spawn(async move {
                while let Some((conn, control)) = q.accept().await {
                    tokio::spawn(serve(conn, control, lists.clone()));
                }
            });
        }
        let list = ServerList {
            servers: vec![app_online::ServerEntry {
                id: "lo".into(),
                name: "Loopback".into(),
                game: "d2rust".into(),
                address: quic.local_addr().unwrap().to_string(),
                trust: ServerTrust::Pinned(fingerprint(&cert)),
                version: d2_client::launch::VERSION.into(),
            }],
        };

        // The host handles, the screens on the game link.
        let h = StartHandles::default();
        let servers = entries(&list);
        *h.online.borrow_mut() = Some(Online::new(
            Box::new(NetLink::new(h.session.clone(), list)),
            servers,
        ));
        let dir = std::env::temp_dir().join(format!("d2-fe-loop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let open = |entry| {
            let mut host = front_host_with(&dir, None, true, entry, &h);
            std::mem::replace(
                &mut host.front,
                FrontEnd::new(true, Box::new(false), Registry::default()),
            )
        };
        let online = h.online.borrow().clone().unwrap();
        let mut r = Rig {
            f: open(Entry::Online),
            server: SharedFake(std::rc::Rc::default()),
            online: online.clone(),
        };
        assert_eq!(r.f.current(), ONLINE_LOGIN);
        r.typed("mogger");
        r.key(TAB);
        r.typed("password1");
        r.key(ENTER);
        until(&mut r, "the character list", |r| {
            r.ids().contains(&"online_chars.char(Sorc)".to_string())
        });
        r.click("online_chars.char(Sorc)");
        r.click("online_chars.ok");
        until(&mut r, "the lobby list", |r| {
            r.f.current() == ONLINE_LOBBY
                && r.ids().contains(&"online_lobby.game(loop)".to_string())
        });
        r.click("online_lobby.game(loop)");
        r.click("online_lobby.primary");
        until(
            &mut r,
            "the game load",
            |r| matches!(r.f.outcome(), Some(Outcome::GameLoad(g)) if g.online),
        );

        // The game start: the join answer comes on a worker thread.
        let join = OnlineJoin::take(&mut online.borrow_mut()).expect("the lobby join");
        assert_eq!(
            (join.ticket, join.name.as_str(), join.class),
            (TICKET, "Sorc", 1)
        );
        let asked = Instant::now();
        let task = JoinTask::start(&h.session, join.ticket).unwrap();
        assert!(task.poll().is_none(), "the join answered before the server");
        assert!(
            asked.elapsed() < ATTACH_DELAY,
            "starting the join waited for the server"
        );
        let end = Instant::now() + Duration::from_secs(10);
        let game = loop {
            if let Some(a) = task.poll() {
                break a.expect("the game welcome");
            }
            assert!(Instant::now() < end, "no answer to the join");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(game.welcome().resume, GAME_TOKEN);
        drop(game);

        // After the game: the lobby, on the same session.
        assert_eq!(after_online_game(&h, None), Entry::Lobby);
        let before = lists.load(Ordering::SeqCst);
        r.f = open(Entry::Lobby);
        assert_eq!(r.f.current(), ONLINE_LOBBY);
        until(&mut r, "the lobby listed again", |r| {
            lists.load(Ordering::SeqCst) > before
                && r.ids().contains(&"online_lobby.game(loop)".to_string())
        });

        // The session gone: the login, with why.
        if let Some(s) = h.session.0.lock().unwrap().take() {
            s.close();
        }
        assert_eq!(
            after_online_game(&h, Some("The server is gone.".into())),
            Entry::OnlineError
        );
        r.f = open(Entry::OnlineError);
        r.ticks(1);
        assert_eq!(r.f.current(), ONLINE_LOGIN);
        assert!(r.shows("The server is gone."));
        quic.close();
    }
}
