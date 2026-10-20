//! The online login screen: the main menu's background and burning logo,
//! the server (a button that steps through `servers.toml`'s list, drawn as
//! the gateway button is), account name and password, LOG IN, NEW ACCOUNT
//! (adds a confirm field and creates the account) and BACK.
//!
//! LOG IN connects first when there is no connection yet, then sends the
//! login; a wait box with CANCEL is up meanwhile. A failure shows its text
//! in a message box and the typed name stays. The account name and password
//! rules are checked before anything is sent, with the same texts the
//! server's errors give.

use crate::ui::front_end::control::vk;
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screens::main_menu;
use crate::ui::front_end::{Action, Control, ControlKind, FrontCtx, Registry, Screen};

use super::fake::account_name_ok;
use super::link::{failure_text, ErrorCode, Event, Failure, Request};
use super::widgets::*;
use super::{OnlineHandle, ONLINE_LOGIN};

/// Custom action ids.
pub mod act {
    pub const FIELD: u32 = 1;
    pub const LOGIN: u32 = 10;
    pub const NEW_ACCOUNT: u32 = 11;
    pub const SERVER: u32 = 12;
    pub const BACK: u32 = 13;
    pub const MESSAGE_OK: u32 = 14;
    pub const CANCEL: u32 = 15;
}

/// The fields: account, password, confirm (new account only).
const ACCOUNT: usize = 0;
const PASSWORD: usize = 1;
const CONFIRM: usize = 2;

/// Field column and the fields' cel bottoms.
const FIELD_X: i32 = 316;
const FIELD_Y: [i32; 3] = [392, 442, 492];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    /// The handshake is out; `then` goes once it is done.
    Connecting {
        then: Request,
    },
    /// A login or register is out.
    Waiting,
}

pub struct Login {
    online: OnlineHandle,
    fields: [Field; 3],
    focus: usize,
    register: bool,
    phase: Phase,
    message: Option<String>,
    rebuild: bool,
    /// The fields' text controls in the built list.
    text_ix: Vec<usize>,
    now_ms: u64,
}

impl Login {
    pub fn new(online: OnlineHandle) -> Self {
        Login {
            online,
            fields: [Field::new(15), Field::password(64), Field::password(64)],
            focus: ACCOUNT,
            register: false,
            phase: Phase::Idle,
            message: None,
            rebuild: false,
            text_ix: Vec::new(),
            now_ms: 0,
        }
    }

    fn again(&mut self) -> Option<Trigger> {
        self.rebuild = true;
        Some(Trigger::Rebuild)
    }

    fn fields_shown(&self) -> usize {
        if self.register {
            3
        } else {
            2
        }
    }

    /// LOG IN (or CREATE ACCOUNT): the client-side rules, then the request
    /// (after a handshake when there is no connection).
    fn submit(&mut self) -> Option<Trigger> {
        if self.phase != Phase::Idle || self.message.is_some() {
            return None;
        }
        let account = self.fields[ACCOUNT].text.clone();
        let password = self.fields[PASSWORD].text.clone();
        let fail = |code: ErrorCode| {
            Some(failure_text(&Failure {
                code,
                wait_s: 0,
                text: String::new(),
            }))
        };
        let (no_servers, connected, registration_open, index) = {
            let st = &self.online.borrow().state;
            (
                st.servers.is_empty(),
                st.connected,
                st.registration_open,
                st.server,
            )
        };
        self.message = if no_servers {
            Some(NO_SERVERS.into())
        } else if account.is_empty() {
            Some("Type your account name.".into())
        } else if password.is_empty() {
            Some("Type your password.".into())
        } else if self.register && !account_name_ok(&account) {
            fail(ErrorCode::BAD_ACCOUNT_NAME)
        } else if self.register && !(8..=64).contains(&password.len()) {
            fail(ErrorCode::BAD_PASSWORD)
        } else if self.register && self.fields[CONFIRM].text != password {
            Some("The two passwords are not the same.".into())
        } else if self.register && connected && !registration_open {
            fail(ErrorCode::REGISTRATION_CLOSED)
        } else {
            None
        };
        if self.message.is_some() {
            return self.again();
        }
        let request = if self.register {
            Request::Register { account, password }
        } else {
            Request::Login { account, password }
        };
        let mut o = self.online.borrow_mut();
        if connected {
            o.send(request);
            self.phase = Phase::Waiting;
        } else {
            o.send(Request::Connect { index });
            self.phase = Phase::Connecting { then: request };
        }
        drop(o);
        self.again()
    }

    fn wait_text(&self) -> String {
        match &self.phase {
            Phase::Connecting { .. } => {
                let st = &self.online.borrow().state;
                let name = st
                    .servers
                    .get(st.server)
                    .map_or("the server", |s| s.name.as_str());
                format!("Connecting to {name}...")
            }
            _ if self.register => "Creating the account...".into(),
            _ => "Logging in...".into(),
        }
    }
}

/// The login screen's text when `servers.toml` is missing.
pub const NO_SERVERS: &str =
    "No server list. Start the game once from the launcher to get the server list.";

impl Screen for Login {
    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        self.now_ms = ctx.now_ms;
        if !std::mem::take(&mut self.rebuild) {
            // A fresh entry: the account name stays (typed before), the
            // passwords go; a notice from the last screen shows.
            self.fields[PASSWORD].text.clear();
            self.fields[CONFIRM].text.clear();
            self.focus = if self.fields[ACCOUNT].text.is_empty() {
                ACCOUNT
            } else {
                PASSWORD
            };
            self.register = false;
            self.phase = Phase::Idle;
            self.message = self.online.borrow_mut().state.notice.take();
            if self.message.is_none() && self.online.borrow().state.servers.is_empty() {
                self.message = Some(NO_SERVERS.into());
            }
        }
        let st = self.online.borrow().state.clone();
        let mut v = vec![
            Control::new(ControlKind::Image, 0, 599, 800, 600).with_art(if ctx.expansion {
                BG_MENU
            } else {
                BG_MENU_CLASSIC
            }),
            Control::new(ControlKind::AnimImage, 400, 120, 181, 170).with_art(main_menu::LOGO_LEFT),
            Control::new(ControlKind::AnimImage, 400, 120, 188, 177)
                .with_art(main_menu::LOGO_RIGHT),
            centred(
                0,
                300,
                800,
                40,
                FONT30,
                if self.register {
                    "NEW ACCOUNT"
                } else {
                    "LOG IN"
                },
            ),
        ];
        // The server: the gateway button's art and place, one step on per
        // click when there are several.
        let server = st
            .servers
            .get(st.server)
            .map_or("none".to_string(), |s| s.name.clone());
        let mut srv = Control::new(ControlKind::Button, 264, 336, 272, 25)
            .with_art(main_menu::NARROW)
            .with_text(format!("SERVER: {}", server.to_uppercase()), 0)
            .with_action(Action::Custom(act::SERVER));
        srv.enabled = st.servers.len() > 1 && !st.connected;
        v.push(srv);
        let names = ["ACCOUNT NAME", "PASSWORD", "CONFIRM PASSWORD"];
        self.text_ix.clear();
        for i in 0..self.fields_shown() {
            v.push(label(FIELD_X, FIELD_Y[i] - 28, 200, names[i], GOLD));
            v.extend(field_controls(FIELD_X, FIELD_Y[i], act::FIELD + i as u32));
            self.text_ix.push(v.len() - 1);
        }
        let mut new = Control::new(ControlKind::Button, 264, 540, 272, 35)
            .with_art(WIDE)
            .with_text(
                if self.register {
                    "BACK TO LOG IN"
                } else {
                    "NEW ACCOUNT"
                },
                0,
            )
            .with_action(Action::Custom(act::NEW_ACCOUNT));
        new.enabled = !st.connected || st.registration_open || self.register;
        v.push(new);
        v.push(button(33, 572, 128, 35, MEDIUM, "BACK", act::BACK).with_hotkey(vk::ESC));
        let mut ok = button(
            627,
            572,
            128,
            35,
            MEDIUM,
            if self.register { "CREATE" } else { "LOG IN" },
            act::LOGIN,
        )
        .with_hotkey(vk::ENTER);
        ok.enabled = !st.servers.is_empty();
        v.push(ok);
        if let Some(m) = &self.message {
            push_message(&mut v, SKY_POPUP, m, act::MESSAGE_OK);
        } else if self.phase != Phase::Idle {
            push_wait(&mut v, SKY_POPUP, &self.wait_text(), act::CANCEL);
        }
        modal(&mut v);
        v
    }

    fn action(&mut self, _ctx: &mut FrontCtx, id: u32) -> Option<Trigger> {
        match id {
            act::LOGIN => self.submit(),
            act::BACK => {
                if self.online.borrow().state.connected {
                    self.online.borrow_mut().send(Request::Logout);
                    let mut o = self.online.borrow_mut();
                    o.state.connected = false;
                }
                Some(Trigger::Exit)
            }
            act::NEW_ACCOUNT => {
                self.register = !self.register;
                self.fields[CONFIRM].text.clear();
                if self.focus == CONFIRM {
                    self.focus = PASSWORD;
                }
                self.again()
            }
            act::SERVER => {
                let mut o = self.online.borrow_mut();
                let n = o.state.servers.len().max(1);
                o.state.server = (o.state.server + 1) % n;
                drop(o);
                self.again()
            }
            act::MESSAGE_OK => {
                self.message = None;
                self.again()
            }
            act::CANCEL => {
                // Drop the connection: a late answer must not log in.
                self.online.borrow_mut().send(Request::Logout);
                self.online.borrow_mut().state.connected = false;
                self.phase = Phase::Idle;
                self.again()
            }
            f if (act::FIELD..act::FIELD + 3).contains(&f) => {
                self.focus = (f - act::FIELD) as usize;
                None
            }
            _ => None,
        }
    }

    fn tick(&mut self, ctx: &mut FrontCtx) -> Option<Trigger> {
        self.now_ms = ctx.now_ms;
        let events = self.online.borrow_mut().take_events();
        let mut out = None;
        for ev in events {
            match ev {
                Event::Connected { .. } => {
                    if let Phase::Connecting { then } =
                        std::mem::replace(&mut self.phase, Phase::Waiting)
                    {
                        let closed = !self.online.borrow().state.registration_open;
                        if matches!(then, Request::Register { .. }) && closed {
                            self.phase = Phase::Idle;
                            self.message = Some(failure_text(&Failure {
                                code: ErrorCode::REGISTRATION_CLOSED,
                                wait_s: 0,
                                text: String::new(),
                            }));
                        } else {
                            self.online.borrow_mut().send(then);
                        }
                    }
                    out = self.again();
                }
                Event::LoggedIn { .. } if self.phase == Phase::Waiting => {
                    self.phase = Phase::Idle;
                    self.fields[PASSWORD].text.clear();
                    self.fields[CONFIRM].text.clear();
                    return Some(Trigger::Ok);
                }
                Event::Error(f) if self.phase != Phase::Idle => {
                    self.phase = Phase::Idle;
                    self.message = Some(failure_text(&f));
                    out = self.again();
                }
                Event::Disconnected(_) => {
                    self.phase = Phase::Idle;
                    self.message = self.online.borrow_mut().state.notice.take();
                    out = self.again();
                }
                _ => {}
            }
        }
        out
    }

    fn char(&mut self, _ctx: &mut FrontCtx, unit: u16) {
        if self.phase != Phase::Idle || self.message.is_some() {
            return;
        }
        let Some(c) = char::from_u32(u32::from(unit)) else {
            return;
        };
        let f = &mut self.fields[self.focus];
        if self.focus == ACCOUNT {
            f.type_char(c, |c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        } else {
            f.type_char(c, |_| true);
        }
    }

    fn key(&mut self, _ctx: &mut FrontCtx, k: u16) -> Option<Trigger> {
        if self.phase != Phase::Idle || self.message.is_some() {
            return None;
        }
        match k {
            vk::BACKSPACE => self.fields[self.focus].backspace(),
            vk::TAB => self.focus = (self.focus + 1) % self.fields_shown(),
            _ => {}
        }
        None
    }

    fn sync(&mut self, controls: &mut [Control]) {
        let live = self.message.is_none() && self.phase == Phase::Idle;
        for (i, &ix) in self.text_ix.iter().enumerate() {
            if let Some(c) = controls.get_mut(ix) {
                show_field(c, &self.fields[i], live && i == self.focus, self.now_ms, 14);
            }
        }
    }
}

pub fn register(reg: &mut Registry, online: OnlineHandle) {
    reg.register(ONLINE_LOGIN, Box::new(Login::new(online)));
}
