//! The lobby, on the menus' lobby art (`realmbckg`): chat in the large pane
//! on the left with its input line below it; on the right the selected
//! character (name, level and class, which difficulties it unlocked), the
//! CREATE and GAMES tabs, and the panel under them: the game list or
//! the create-game form. The bottom slots hold BACK (or CANCEL in the form)
//! and JOIN (CREATE in the form).
//!
//! - **Game list**: name, difficulty, players / most players, `PW` for a
//!   password game. Games the character can't join are grey; the line above
//!   the list says why (the server's `joinable` code). Click selects, a
//!   double click or JOIN joins; Up / Down / wheel move and scroll.
//! - **Join**: a password game opens the password prompt first; a blank
//!   password is refused before anything is sent.
//! - **Create**: name, password (blank = a public game) and difficulty.
//!   Only the difficulties the character unlocked can be picked (the same
//!   rule the server applies, `d2_formats::d2s::status::difficulties`).
//! - **Chat**: Enter sends the typed line (JOIN's Enter sends chat when
//!   a line is typed). A server without chat hides the input line.
//!
//! Every failure shows its text in the message box; the screen stays as it
//! was. JoinOk ends the front end in an online game load.

use d2_formats::d2s::status;

use crate::ui::front_end::control::vk;
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screens::char_select::class_name;
use crate::ui::front_end::{Action, Control, ControlKind, DrawItem, FrontCtx, Registry, Screen};
use crate::ui::geom::{Point, Rect};
use crate::ui::ui_ids::UiElement;

use super::fake::game_name_ok;
use super::link::{
    difficulty_name, failure_text, CharInfo, ErrorCode, Event, Failure, GameInfo, Request,
};
use super::widgets::*;
use super::{OnlineHandle, ONLINE_LOBBY};

/// The lobby art (800 × 600, 4 × 3 tiles).
pub const BG: &str = r"FrontEnd\realmbckg";

/// Custom action ids.
pub mod act {
    pub const BACK: u32 = 1;
    pub const PRIMARY: u32 = 2;
    pub const TAB_CREATE: u32 = 3;
    pub const TAB_JOIN: u32 = 4;
    pub const MESSAGE_OK: u32 = 5;
    pub const CANCEL: u32 = 6;
    pub const PROMPT_OK: u32 = 7;
    pub const PROMPT_CANCEL: u32 = 8;
    pub const SCROLL_UP: u32 = 9;
    pub const SCROLL_DOWN: u32 = 10;
    pub const FIELD: u32 = 20;
    pub const DIFFICULTY: u32 = 30;
}

/// The chat pane (inside the art's left frame) and its input line.
const CHAT: Rect = Rect {
    x: 68,
    y: 186,
    w: 324,
    h: 264,
};
const CHAT_ROW: i32 = 16;
/// The input line's text control (inside the art's box, 478-502; its bottom
/// kept above the footer rows that move with the window's height).
const CHAT_INPUT: (i32, i32) = (117, 499);
/// The right panel: the list or the form.
const PANEL: Rect = Rect {
    x: 450,
    y: 316,
    w: 312,
    h: 184,
};
/// List rows: a header, then [`ROWS`] rows of [`ROW_H`].
pub const ROWS: usize = 8;
const ROW_H: i32 = 20;
const LIST_TOP: i32 = 338;
/// A row's frame and click area start this far above the row's text box
/// (the text sits at the top of its box).
const ROW_LIFT: i32 = 4;
/// Columns: name, difficulty, players, password mark (no header).
const COLS: [i32; 4] = [454, 576, 672, 740];
/// The form's fields (cel bottoms) and difficulty row.
const FORM_X: i32 = 584;
const FORM_NAME_Y: i32 = 350;
const FORM_PASS_Y: i32 = 392;
const FORM_DIFF_Y: i32 = 440;
/// The password prompt's field.
const PROMPT: (i32, i32) = (315, 300);
/// Double click.
const DOUBLE_MS: u64 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    List,
    Create,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    /// A create or join is out.
    Waiting(String),
    /// The password prompt for joining this game.
    Password(u32),
}

/// The lobby's state, open for tests.
pub struct Lobby {
    online: OnlineHandle,
    pub view: View,
    phase: Phase,
    message: Option<String>,
    /// The selected game's id.
    pub selected: Option<u32>,
    /// The first list row shown.
    pub first: usize,
    chat: Field,
    game_name: Field,
    game_password: Field,
    join_password: Field,
    /// The form's focus: 0 name, 1 password.
    focus: usize,
    pub difficulty: u8,
    last_click: Option<(u64, u32)>,
    now_ms: u64,
    rebuild: bool,
    /// Games and chat as last built (a change rebuilds).
    seen: (usize, usize),
    listed: bool,
    /// Text controls of the fields in the built list: chat input, game
    /// name, game password, join password.
    text_ix: [Option<usize>; 4],
}

impl Lobby {
    pub fn new(online: OnlineHandle) -> Self {
        Lobby {
            online,
            view: View::List,
            phase: Phase::Idle,
            message: None,
            selected: None,
            first: 0,
            chat: Field::new(200),
            game_name: Field::new(15),
            game_password: Field::new(15),
            join_password: Field::password(15),
            focus: 0,
            difficulty: 0,
            last_click: None,
            now_ms: 0,
            rebuild: false,
            seen: (0, 0),
            listed: false,
            text_ix: [None; 4],
        }
    }

    fn again(&mut self) -> Option<Trigger> {
        self.rebuild = true;
        Some(Trigger::Rebuild)
    }

    fn modal_up(&self) -> bool {
        self.message.is_some() || self.phase != Phase::Idle
    }

    fn character(&self) -> Option<CharInfo> {
        self.online.borrow().state.selected.clone()
    }

    /// The games in list order (the server's order: oldest first).
    fn games(&self) -> Vec<GameInfo> {
        self.online.borrow().state.games.clone()
    }

    fn selected_game(&self) -> Option<GameInfo> {
        let id = self.selected?;
        self.games().into_iter().find(|g| g.id == id)
    }

    fn mask(&self) -> u8 {
        self.character()
            .map_or(status::NORMAL_BIT, |c| c.difficulties)
    }

    fn fail(&mut self, code: ErrorCode) -> Option<Trigger> {
        self.message = Some(failure_text(&Failure {
            code,
            wait_s: 0,
            text: String::new(),
        }));
        self.again()
    }

    /// JOIN on the selected game: the client's own checks first (the
    /// server's rules, so the player hears the reason at once), then the
    /// password prompt for a password game, then the request.
    fn join(&mut self) -> Option<Trigger> {
        let Some(g) = self.selected_game() else {
            self.message = Some("Choose a game in the list first.".into());
            return self.again();
        };
        if g.joinable != 0 {
            return self.fail(ErrorCode(g.joinable));
        }
        if g.locked {
            self.join_password.text.clear();
            self.phase = Phase::Password(g.id);
            return self.again();
        }
        self.send_join(g.id, String::new())
    }

    fn send_join(&mut self, game_id: u32, password: String) -> Option<Trigger> {
        let name = self.selected_game().map(|g| g.name).unwrap_or_default();
        self.online
            .borrow_mut()
            .send(Request::JoinGame { game_id, password });
        self.phase = Phase::Waiting(format!("Joining {name}..."));
        self.again()
    }

    /// CREATE in the form.
    fn create(&mut self) -> Option<Trigger> {
        let name = self.game_name.text.trim().to_string();
        if name.is_empty() {
            self.message = Some("Type a name for the game.".into());
            return self.again();
        }
        if !game_name_ok(&name) {
            return self.fail(ErrorCode::BAD_GAME_NAME);
        }
        if !status::difficulty_open(self.mask(), self.difficulty) {
            return self.fail(ErrorCode::DIFFICULTY_LOCKED);
        }
        self.online.borrow_mut().send(Request::CreateGame {
            name: name.clone(),
            password: self.game_password.text.clone(),
            difficulty: self.difficulty,
        });
        self.phase = Phase::Waiting(format!("Creating {name}..."));
        self.again()
    }

    fn send_chat(&mut self) -> Option<Trigger> {
        let mut text = self.chat.text.trim().to_string();
        while text.len() > 200 {
            text.pop();
        }
        self.chat.text.clear();
        if !text.is_empty() {
            self.online.borrow_mut().send(Request::Chat { text });
        }
        None
    }

    /// Move the selection by `d` rows (keys), keeping it in view.
    fn step(&mut self, d: i32) -> Option<Trigger> {
        let games = self.games();
        if games.is_empty() {
            return None;
        }
        let at = self
            .selected
            .and_then(|id| games.iter().position(|g| g.id == id))
            .map_or(0, |i| {
                (i as i32 + d).clamp(0, games.len() as i32 - 1) as usize
            });
        self.selected = Some(games[at].id);
        if at < self.first {
            self.first = at;
        } else if at >= self.first + ROWS {
            self.first = at + 1 - ROWS;
        }
        self.again()
    }

    fn scroll(&mut self, d: i32) -> Option<Trigger> {
        let n = self.games().len();
        let max = n.saturating_sub(ROWS);
        self.first = (self.first as i32 + d).clamp(0, max as i32) as usize;
        self.again()
    }

    /// The list row under `p`.
    fn row_at(&self, p: Point) -> Option<usize> {
        let top = LIST_TOP - ROW_LIFT;
        if p.x < PANEL.x || p.x >= PANEL.x + PANEL.w as i32 - 14 || p.y < top {
            return None;
        }
        let r = ((p.y - top) / ROW_H) as usize;
        (r < ROWS).then_some(self.first + r)
    }

    fn info_lines(&self) -> Vec<(String, i32)> {
        let Some(c) = self.character() else {
            return Vec::new();
        };
        let mut kind = String::new();
        if c.status & status::HARDCORE != 0 {
            kind.push_str("Hardcore ");
        }
        if c.status & status::EXPANSION != 0 {
            kind.push_str("Expansion ");
        }
        let mut lines = vec![
            (c.name.clone(), GOLD),
            (
                format!(
                    "Level {} {} {}",
                    c.level,
                    class_name(c.class),
                    kind.trim_end()
                )
                .trim_end()
                .to_string(),
                WHITE,
            ),
        ];
        let open: Vec<&str> = (0..3)
            .filter(|&d| status::difficulty_open(c.difficulties, d))
            .map(difficulty_name)
            .collect();
        lines.push((format!("Difficulties: {}", open.join(", ")), WHITE));
        if self.view == View::List {
            match self.selected_game() {
                Some(g) if g.joinable != 0 => lines.push((list_reason(&g), RED)),
                Some(g) if g.locked => lines.push((format!("{}: password game", g.name), GOLD)),
                _ => {}
            }
        }
        lines
    }

    fn list_controls(&self, v: &mut Vec<Control>) {
        let games = self.games();
        for (x, h) in COLS.iter().zip(["NAME", "DIFFICULTY", "PLAYERS"]) {
            v.push(label(*x, LIST_TOP - 2, 100, h, GOLD));
        }
        if games.is_empty() {
            let text = if self.listed {
                "No open games. Create one!"
            } else {
                "Getting the game list..."
            };
            v.push(centred(PANEL.x, LIST_TOP + 60, PANEL.w, 20, FONT16, text));
        }
        for (r, g) in games.iter().skip(self.first).take(ROWS).enumerate() {
            let y = LIST_TOP + ROW_H * (r as i32 + 1) - 2;
            let colour = if g.joinable != 0 { GREY } else { WHITE };
            v.push(label(COLS[0], y, 144, g.name.clone(), colour));
            v.push(label(COLS[1], y, 84, difficulty_name(g.difficulty), colour));
            v.push(label(
                COLS[2],
                y,
                46,
                format!("{}/{}", g.players, g.max_players),
                colour,
            ));
            if g.locked {
                v.push(label(COLS[3], y, 26, "PW", GOLD));
            }
        }
        if games.len() > ROWS {
            let cel = |frame: u32, top: i32, id: Option<u32>| {
                let mut c =
                    Control::new(ControlKind::Image, 748, top + 14, 12, 14).with_art(SCROLL);
                c.cel = Some(frame);
                if let Some(id) = id {
                    c.action = Action::Custom(id);
                }
                c
            };
            let range = (games.len() - ROWS) as i32;
            v.push(cel(0, LIST_TOP, Some(act::SCROLL_UP)));
            v.push(cel(
                1,
                LIST_TOP + ROW_H * ROWS as i32 - 14,
                Some(act::SCROLL_DOWN),
            ));
            let track = ROW_H * ROWS as i32 - 14 * 3;
            v.push(cel(
                4,
                LIST_TOP + 14 + track * self.first as i32 / range,
                None,
            ));
        }
    }

    fn form_controls(&mut self, v: &mut Vec<Control>) {
        v.push(label(PANEL.x + 4, FORM_NAME_Y - 4, 130, "GAME NAME", GOLD));
        v.extend(field_controls(FORM_X, FORM_NAME_Y, act::FIELD));
        self.text_ix[1] = Some(v.len() - 1);
        v.push(label(PANEL.x + 4, FORM_PASS_Y - 4, 130, "PASSWORD", GOLD));
        v.extend(field_controls(FORM_X, FORM_PASS_Y, act::FIELD + 1));
        self.text_ix[2] = Some(v.len() - 1);
        v.push(label(
            PANEL.x + 4,
            FORM_PASS_Y + 18,
            300,
            "Blank: a public game.",
            WHITE,
        ));
        v.push(label(PANEL.x + 4, FORM_DIFF_Y, 130, "DIFFICULTY", GOLD));
        let mask = self.mask();
        for d in 0..3u8 {
            let x = PANEL.x + 4 + 104 * i32::from(d);
            let y = FORM_DIFF_Y + 26;
            let open = status::difficulty_open(mask, d);
            let mut r = Control::new(ControlKind::Button, x, y, 16, 16)
                .with_art(RADIO)
                .with_action(Action::Custom(act::DIFFICULTY + u32::from(d)))
                .with_on_press();
            r.checked = self.difficulty == d;
            r.enabled = open;
            v.push(r);
            v.push(label(
                x + 20,
                y + 1,
                84,
                difficulty_name(d),
                if open { WHITE } else { GREY },
            ));
        }
        if mask != 0b111 {
            v.push(label(
                PANEL.x + 4,
                FORM_DIFF_Y + 50,
                300,
                "Grey: not unlocked by this character yet.",
                GREY,
            ));
        }
    }
}

impl Screen for Lobby {
    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        if !std::mem::take(&mut self.rebuild) {
            // A fresh entry (from the character list, or back from a game).
            self.view = View::List;
            self.phase = Phase::Idle;
            self.selected = None;
            self.first = 0;
            self.chat.text.clear();
            self.listed = false;
            self.difficulty = (0..3)
                .rev()
                .find(|&d| status::difficulty_open(self.mask(), d))
                .unwrap_or(0);
            let mut o = self.online.borrow_mut();
            self.message = o.state.notice.take();
            o.state.games.clear();
            o.send(Request::ListGames);
        }
        let st = self.online.borrow().state.clone();
        self.seen = (st.games.len(), st.chat.len());
        self.text_ix = [None; 4];
        self.now_ms = ctx.now_ms;
        let mut v = vec![
            Control::new(ControlKind::Image, 0, 599, 800, 600).with_art(BG),
            centred(0, 70, 800, 40, FONT30, st.server_name.clone()),
            centred(
                0,
                98,
                800,
                24,
                FONT16,
                format!("Account {}", st.account.clone().unwrap_or_default()),
            ),
        ];
        for (i, (text, colour)) in self.info_lines().into_iter().enumerate() {
            v.push(label(450, 200 + 18 * i as i32, 316, text, colour));
        }
        // The tab of the panel shown is off.
        let mut tab = |x: i32, text: &str, id: u32, on: bool| {
            let mut b = button(x, 305, 128, 35, MEDIUM, text, id);
            b.enabled = !on;
            v.push(b);
        };
        tab(446, "CREATE", act::TAB_CREATE, self.view == View::Create);
        tab(638, "GAMES", act::TAB_JOIN, self.view == View::List);
        match self.view {
            View::List => self.list_controls(&mut v),
            View::Create => self.form_controls(&mut v),
        }
        if !st.chat_off && self.view == View::List {
            v.push(field_text(CHAT_INPUT.0, CHAT_INPUT.1));
            self.text_ix[0] = Some(v.len() - 1);
        }
        if st.chat_off {
            v.push(label(
                CHAT_INPUT.0,
                CHAT_INPUT.1,
                270,
                "Chat is not available on this server.",
                GREY,
            ));
        }
        let (back, primary) = match self.view {
            View::List => ("BACK", "JOIN"),
            View::Create => ("CANCEL", "CREATE"),
        };
        v.push(button(38, 570, 128, 35, MEDIUM, back, act::BACK).with_hotkey(vk::ESC));
        v.push(button(635, 570, 128, 35, MEDIUM, primary, act::PRIMARY).with_hotkey(vk::ENTER));
        if let Some(m) = &self.message {
            push_message(&mut v, SKY_POPUP, m, act::MESSAGE_OK);
        } else {
            match &self.phase {
                Phase::Idle => {}
                Phase::Waiting(text) => push_wait(&mut v, SKY_POPUP, text, act::CANCEL),
                Phase::Password(_) => {
                    push_popup_art(&mut v, SKY_POPUP);
                    let mut t = popup_text("This game needs a password.");
                    t.h = 50;
                    t.y = 270;
                    v.push(t);
                    v.extend(field_controls(PROMPT.0, PROMPT.1, act::FIELD + 2));
                    self.text_ix[3] = Some(v.len() - 1);
                    v.push(
                        button(
                            281,
                            337,
                            96,
                            32,
                            SKY_POPUP.button,
                            "CANCEL",
                            act::PROMPT_CANCEL,
                        )
                        .with_hotkey(vk::ESC)
                        .with_on_press(),
                    );
                    v.push(
                        button(421, 337, 96, 32, SKY_POPUP.button, "JOIN", act::PROMPT_OK)
                            .with_hotkey(vk::ENTER)
                            .with_on_press(),
                    );
                }
            }
        }
        modal(&mut v);
        v
    }

    fn action(&mut self, _ctx: &mut FrontCtx, id: u32) -> Option<Trigger> {
        match id {
            act::MESSAGE_OK => {
                self.message = None;
                self.again()
            }
            act::CANCEL => {
                // The answer may still come: JoinOk then still starts the
                // game (the server holds the seat); an error is shown.
                self.phase = Phase::Idle;
                self.again()
            }
            act::PROMPT_CANCEL => {
                self.phase = Phase::Idle;
                self.again()
            }
            act::PROMPT_OK => {
                let Phase::Password(game) = self.phase else {
                    return None;
                };
                if self.join_password.text.is_empty() {
                    self.phase = Phase::Idle;
                    return self.fail(ErrorCode::PASSWORD_NEEDED);
                }
                let pw = std::mem::take(&mut self.join_password.text);
                self.send_join(game, pw)
            }
            act::BACK => match self.view {
                View::Create => {
                    self.view = View::List;
                    self.again()
                }
                View::List => {
                    self.online.borrow_mut().send(Request::LeaveLobby);
                    Some(Trigger::Exit)
                }
            },
            act::PRIMARY => match self.view {
                View::List if !self.chat.text.trim().is_empty() => self.send_chat(),
                View::List => self.join(),
                View::Create => self.create(),
            },
            act::TAB_CREATE => {
                self.view = View::Create;
                self.focus = 0;
                self.again()
            }
            act::TAB_JOIN => {
                self.view = View::List;
                self.again()
            }
            act::SCROLL_UP => self.scroll(-1),
            act::SCROLL_DOWN => self.scroll(1),
            f if (act::FIELD..act::FIELD + 2).contains(&f) => {
                self.focus = (f - act::FIELD) as usize;
                None
            }
            d if (act::DIFFICULTY..act::DIFFICULTY + 3).contains(&d) => {
                let d = (d - act::DIFFICULTY) as u8;
                if status::difficulty_open(self.mask(), d) {
                    self.difficulty = d;
                }
                self.again()
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
                Event::GameListEnd => self.listed = true,
                Event::GameGone(id) if self.selected == Some(id) => self.selected = None,
                Event::JoinOk(_) => {
                    self.phase = Phase::Idle;
                    self.online.borrow_mut().send(Request::LeaveLobby);
                    return Some(Trigger::Ok);
                }
                Event::Error(f) if f.code == ErrorCode::NOT_SUPPORTED => {}
                Event::Error(f) => {
                    self.phase = Phase::Idle;
                    self.message = Some(failure_text(&f));
                }
                Event::Disconnected(_) => return Some(Trigger::OnlineLost),
                _ => {}
            }
            out = Some(());
        }
        let now = {
            let st = &self.online.borrow().state;
            (st.games.len(), st.chat.len())
        };
        if out.is_some() || now != self.seen {
            return self.again();
        }
        None
    }

    fn char(&mut self, _ctx: &mut FrontCtx, unit: u16) {
        let Some(c) = char::from_u32(u32::from(unit)) else {
            return;
        };
        if let Phase::Password(_) = self.phase {
            if self.message.is_none() {
                self.join_password.type_char(c, |_| true);
            }
            return;
        }
        if self.modal_up() {
            return;
        }
        match self.view {
            View::Create if self.focus == 0 => {
                self.game_name.type_char(c, |c| {
                    c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_'
                });
            }
            View::Create => {
                self.game_password.type_char(c, |_| true);
            }
            View::List if !self.online.borrow().state.chat_off => {
                self.chat.type_char(c, |_| true);
            }
            View::List => {}
        }
    }

    fn key(&mut self, _ctx: &mut FrontCtx, k: u16) -> Option<Trigger> {
        if let Phase::Password(_) = self.phase {
            if k == vk::BACKSPACE && self.message.is_none() {
                self.join_password.backspace();
            }
            return None;
        }
        if self.modal_up() {
            return None;
        }
        const UP: u16 = 38;
        const DOWN: u16 = 40;
        match (self.view, k) {
            (View::Create, vk::BACKSPACE) if self.focus == 0 => self.game_name.backspace(),
            (View::Create, vk::BACKSPACE) => self.game_password.backspace(),
            (View::Create, vk::TAB) => self.focus = 1 - self.focus,
            (View::List, vk::BACKSPACE) => self.chat.backspace(),
            (View::List, UP) => return self.step(-1),
            (View::List, DOWN) => return self.step(1),
            _ => {}
        }
        None
    }

    fn wheel(&mut self, _ctx: &mut FrontCtx, delta: i32) -> Option<Trigger> {
        if self.modal_up() || self.view != View::List {
            return None;
        }
        self.scroll(-delta.signum())
    }

    fn mouse(&mut self, ctx: &mut FrontCtx, down: bool, p: Point) -> Option<Trigger> {
        if !down || self.modal_up() || self.view != View::List {
            return None;
        }
        let games = self.games();
        let g = games.get(self.row_at(p)?)?;
        let double = self
            .last_click
            .is_some_and(|(t, id)| id == g.id && ctx.now_ms.saturating_sub(t) <= DOUBLE_MS);
        self.last_click = Some((ctx.now_ms, g.id));
        self.selected = Some(g.id);
        if double {
            return self.join();
        }
        self.again()
    }

    fn overlay(&mut self, _now_ms: u64, adv: &dyn Fn(u16, &[u16]) -> i32) -> Vec<DrawItem> {
        let mut out = Vec::new();
        // The selected row's frame (a pop-up draws above the overlay).
        if self.view == View::List {
            if let Some(i) = self
                .selected
                .and_then(|id| self.games().iter().position(|g| g.id == id))
            {
                if i >= self.first && i < self.first + ROWS {
                    out.push(DrawItem::Border {
                        at: Point::new(
                            PANEL.x,
                            LIST_TOP - ROW_LIFT + ROW_H * (i - self.first) as i32,
                        ),
                        w: PANEL.w as i32 - 16,
                        h: ROW_H,
                    });
                }
            }
        }
        // Chat: the newest lines that fit, wrapped to the pane, bottom up.
        let st = self.online.borrow().state.clone();
        let width = |s: &str| adv(FONT16, &s.encode_utf16().collect::<Vec<u16>>());
        let mut rows: Vec<(String, i32)> = Vec::new();
        let mut lines: Vec<(String, i32)> = Vec::new();
        if let Some(c) = &st.selected {
            lines.push((format!("You are in the lobby as {}.", c.name), GOLD));
        }
        for (from, text) in &st.chat {
            lines.push((format!("<{from}> {text}"), WHITE));
        }
        for (line, colour) in lines {
            for r in wrap(&line, CHAT.w as i32, &width) {
                rows.push((r, colour));
            }
        }
        let fit = (CHAT.h as i32 / CHAT_ROW) as usize;
        let skip = rows.len().saturating_sub(fit);
        for (i, (text, colour)) in rows.into_iter().skip(skip).enumerate() {
            out.push(DrawItem::Text {
                string_id: 0,
                text,
                font: FONT16,
                at: Point::new(CHAT.x, CHAT.y + CHAT_ROW * (i as i32 + 1)),
                label: None,
                color: colour,
                boxed: None,
            });
        }
        out
    }

    fn sync(&mut self, controls: &mut [Control]) {
        let live = !self.modal_up();
        let prompt = matches!(self.phase, Phase::Password(_)) && self.message.is_none();
        let fields = [
            (&self.chat, live, 34),
            (&self.game_name, live && self.focus == 0, 15),
            (&self.game_password, live && self.focus == 1, 15),
            (&self.join_password, prompt, 15),
        ];
        for (ix, (f, focused, keep)) in self.text_ix.iter().zip(fields) {
            if let Some(c) = ix.and_then(|i| controls.get_mut(i)) {
                show_field(c, f, focused, self.now_ms, keep);
            }
        }
    }

    fn ui_elements(&self) -> Vec<UiElement> {
        if self.modal_up() || self.view != View::List {
            return Vec::new();
        }
        self.games()
            .iter()
            .enumerate()
            .skip(self.first)
            .take(ROWS)
            .map(|(i, g)| {
                UiElement::new(
                    format!("online_lobby.game({})", g.name),
                    Rect::new(
                        PANEL.x,
                        LIST_TOP - ROW_LIFT + ROW_H * (i - self.first) as i32,
                        PANEL.w - 16,
                        ROW_H as u16,
                    ),
                )
            })
            .collect()
    }
}

/// The short reason above the list why the selected game can't be joined
/// (one row; the message box says it in full).
pub fn list_reason(g: &GameInfo) -> String {
    match ErrorCode(g.joinable) {
        ErrorCode::DIFFICULTY_LOCKED => {
            format!("{} is not unlocked yet.", difficulty_name(g.difficulty))
        }
        ErrorCode::GAME_FULL => "The game is full.".into(),
        ErrorCode::DEAD_HARDCORE => "A dead hardcore character can't join.".into(),
        ErrorCode::WRONG_GAME_KIND => "For another kind of character.".into(),
        c => failure_text(&Failure {
            code: c,
            wait_s: 0,
            text: String::new(),
        }),
    }
}

/// `text` cut into rows no wider than `w` (at spaces; a word wider than a
/// row is cut where it overflows).
pub fn wrap(text: &str, w: i32, width: &dyn Fn(&str) -> i32) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in text.split(' ') {
        let candidate = if row.is_empty() {
            word.to_string()
        } else {
            format!("{row} {word}")
        };
        if width(&candidate) <= w || row.is_empty() && width(word) <= w {
            row = candidate;
            continue;
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        let mut piece = String::new();
        for c in word.chars() {
            piece.push(c);
            if width(&piece) > w {
                piece.pop();
                rows.push(std::mem::take(&mut piece));
                piece.push(c);
            }
        }
        row = piece;
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

pub fn register(reg: &mut Registry, online: OnlineHandle) {
    reg.register(ONLINE_LOBBY, Box::new(Lobby::new(online)));
}
