//! The online character list: the character select screen on the account's
//! characters (the server's list, never the save folder), with the account
//! and server named at the top. OK selects the character on the server and
//! opens the lobby (no difficulty box: the lobby's create-game panel picks
//! the difficulty from what the character unlocked). Create New opens the
//! create screen in online mode. Delete asks the select screen's question,
//! then the account password, and the server deletes. EXIT logs out.

use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screens::char_select::{CharSelect, Entry, SelectionHandle};
use crate::ui::front_end::{Control, DrawItem, FrontCtx, Registry, Screen};
use crate::ui::geom::Point;
use crate::ui::ui_ids::UiElement;

use super::link::{failure_text, CharInfo, Event, Request};
use super::widgets::*;
use super::{OnlineHandle, ONLINE_CHARS};

/// Custom action ids, above the select screen's own.
pub mod act {
    pub const BASE: u32 = 0x1_0000;
    pub const MESSAGE_OK: u32 = BASE + 1;
    pub const CANCEL: u32 = BASE + 2;
    pub const DELETE_OK: u32 = BASE + 3;
    pub const DELETE_CANCEL: u32 = BASE + 4;
    pub const FIELD: u32 = BASE + 5;
}

/// The password prompt's field (the pop-up's text box).
const PROMPT_X: i32 = 315;
const PROMPT_Y: i32 = 300;

/// A server character as the select screen lists it: the appearance bytes
/// are the header's 16 components then 16 colours (the screen draws the
/// first 11 of each); `last_played` orders the list as the save time does.
pub fn entry(c: &CharInfo) -> Entry {
    let mut components = [0xFF; 11];
    let mut colours = [0xFF; 11];
    for (i, b) in c.look.iter().take(11).enumerate() {
        components[i] = *b;
    }
    for (i, b) in c.look.iter().skip(16).take(11).enumerate() {
        colours[i] = *b;
    }
    Entry {
        name: c.name.clone(),
        class: c.class,
        level: c.level,
        status: c.status,
        components,
        colours,
        mtime: u128::from(c.last_played) * 1_000_000_000,
    }
}

/// The select screen's order: the most recently played first (ties keep
/// the server's order).
pub fn entries(chars: &[CharInfo]) -> Vec<Entry> {
    let mut v: Vec<Entry> = chars.iter().map(entry).collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.mtime));
    v
}

const LISTING: &str = "Getting your characters...";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    /// A request is out; the wait box shows the text.
    Waiting(&'static str),
    /// The password prompt for deleting this character.
    DeletePassword(String),
}

pub struct OnlineChars {
    online: OnlineHandle,
    inner: CharSelect,
    handle: SelectionHandle,
    phase: Phase,
    message: Option<String>,
    password: Field,
    rebuild: bool,
    /// The prompt's text control in the built list.
    prompt_ix: Option<usize>,
    now_ms: u64,
}

impl OnlineChars {
    pub fn new(online: OnlineHandle) -> Self {
        let handle = SelectionHandle::default();
        OnlineChars {
            online,
            inner: CharSelect::with_list(Vec::new(), handle.clone()),
            handle,
            phase: Phase::Idle,
            message: None,
            password: Field::password(64),
            rebuild: false,
            prompt_ix: None,
            now_ms: 0,
        }
    }

    pub fn inner(&self) -> &CharSelect {
        &self.inner
    }

    fn again(&mut self) -> Option<Trigger> {
        self.inner.keep();
        self.rebuild = true;
        Some(Trigger::Rebuild)
    }

    /// Rebuild on a new list (the selection starts over).
    fn reload(&mut self, chars: &[CharInfo]) -> Option<Trigger> {
        let t = self.again();
        self.inner.set_list(entries(chars));
        t
    }

    fn modal_up(&self) -> bool {
        self.message.is_some() || self.phase != Phase::Idle
    }

    /// What the inner screen's trigger means online.
    fn route(&mut self, t: Option<Trigger>) -> Option<Trigger> {
        if let Some(name) = self.inner.take_delete_request() {
            self.password.text.clear();
            self.phase = Phase::DeletePassword(name);
            return self.again();
        }
        match t {
            Some(Trigger::Ok) => {
                let sel = self.handle.lock().ok()?.take()?;
                self.online.borrow_mut().send(Request::CharSelect {
                    name: sel.entry.name,
                });
                self.phase = Phase::Waiting("Entering the lobby...");
                self.again()
            }
            Some(Trigger::Exit) => {
                let mut o = self.online.borrow_mut();
                o.send(Request::Logout);
                o.state.connected = false;
                o.state.account = None;
                o.state.chars.clear();
                o.state.selected = None;
                Some(Trigger::Exit)
            }
            Some(Trigger::Rebuild) => {
                // The select screen keeps its model across its own rebuilds.
                self.again()
            }
            other => other,
        }
    }
}

impl Screen for OnlineChars {
    fn canvas_changed(&mut self, canvas: crate::ui::front_end::canvas::FrontCanvas) {
        self.inner.canvas_changed(canvas);
    }

    fn relayout(&mut self, ctx: &mut FrontCtx) -> Option<Vec<Control>> {
        self.rebuild = true;
        Some(self.build(ctx))
    }

    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        if !std::mem::take(&mut self.rebuild) {
            // A fresh entry (from login, back from the lobby or create):
            // show what is known and ask the server for the list.
            self.message = self.online.borrow_mut().state.notice.take();
            let chars = self.online.borrow().state.chars.clone();
            self.inner.set_list(entries(&chars));
            self.online.borrow_mut().send(Request::CharList);
            self.phase = Phase::Waiting(LISTING);
        }
        self.now_ms = ctx.now_ms;
        self.prompt_ix = None;
        let mut v = self.inner.build(ctx);
        let st = self.online.borrow().state.clone();
        // The account and server over the select screen's name line.
        // The account and server, right of the list's frame.
        v.push(
            centred(
                600,
                44,
                200,
                20,
                FONT16,
                st.account.clone().unwrap_or_default(),
            )
            .with_color(GOLD),
        );
        v.push(centred(600, 64, 200, 20, FONT16, st.server_name.clone()));
        if let Some(m) = &self.message {
            push_message(&mut v, SKY_POPUP, m, act::MESSAGE_OK);
        } else {
            match &self.phase {
                Phase::Idle => {}
                Phase::Waiting(text) => push_wait(&mut v, SKY_POPUP, text, act::CANCEL),
                Phase::DeletePassword(name) => {
                    push_popup_art(&mut v, SKY_POPUP);
                    let mut t = popup_text(format!("Type your account password to delete {name}."));
                    t.h = 50;
                    t.y = 270;
                    v.push(t);
                    v.extend(field_controls(PROMPT_X, PROMPT_Y, act::FIELD));
                    self.prompt_ix = Some(v.len() - 1);
                    v.push(
                        button(
                            281,
                            337,
                            96,
                            32,
                            SKY_POPUP.button,
                            "CANCEL",
                            act::DELETE_CANCEL,
                        )
                        .with_hotkey(crate::ui::front_end::control::vk::ESC)
                        .with_on_press(),
                    );
                    v.push(
                        button(421, 337, 96, 32, SKY_POPUP.button, "DELETE", act::DELETE_OK)
                            .with_hotkey(crate::ui::front_end::control::vk::ENTER)
                            .with_on_press(),
                    );
                }
            }
        }
        modal(&mut v);
        v
    }

    fn action(&mut self, ctx: &mut FrontCtx, id: u32) -> Option<Trigger> {
        match id {
            act::MESSAGE_OK => {
                self.message = None;
                self.again()
            }
            act::CANCEL => {
                // Nothing to undo on the server: the answer is dropped.
                self.phase = Phase::Idle;
                self.again()
            }
            act::DELETE_CANCEL => {
                self.phase = Phase::Idle;
                self.again()
            }
            act::DELETE_OK => {
                if let Phase::DeletePassword(name) = &self.phase {
                    let name = name.clone();
                    let password = std::mem::take(&mut self.password.text);
                    self.online
                        .borrow_mut()
                        .send(Request::CharDelete { name, password });
                    self.phase = Phase::Waiting("Deleting the character...");
                }
                self.again()
            }
            act::FIELD => None,
            _ if self.modal_up() => None,
            _ => {
                let t = self.inner.action(ctx, id);
                self.route(t)
            }
        }
    }

    fn tick(&mut self, ctx: &mut FrontCtx) -> Option<Trigger> {
        self.now_ms = ctx.now_ms;
        let t = self.inner.tick(ctx);
        let mut out = self.route(t);
        let events = self.online.borrow_mut().take_events();
        for ev in events {
            match ev {
                Event::Chars(list) => {
                    if self.phase == Phase::Waiting(LISTING) {
                        self.phase = Phase::Idle;
                    }
                    out = self.reload(&list);
                }
                Event::CharDeleted(_) => {
                    let chars = self.online.borrow().state.chars.clone();
                    self.phase = Phase::Idle;
                    out = self.reload(&chars);
                }
                Event::CharSelected(_) => {
                    self.phase = Phase::Idle;
                    return Some(Trigger::Ok);
                }
                Event::Error(f) => {
                    self.phase = Phase::Idle;
                    self.message = Some(failure_text(&f));
                    out = self.again();
                }
                Event::Disconnected(_) => return Some(Trigger::OnlineLost),
                _ => {}
            }
        }
        out
    }

    fn char(&mut self, ctx: &mut FrontCtx, unit: u16) {
        if let Phase::DeletePassword(_) = self.phase {
            if let Some(c) = char::from_u32(u32::from(unit)) {
                self.password.type_char(c, |_| true);
            }
        } else if !self.modal_up() {
            self.inner.char(ctx, unit);
        }
    }

    fn pointer(&mut self, ctx: &mut FrontCtx, p: Point) {
        self.inner.pointer(ctx, p);
    }

    fn wheel(&mut self, ctx: &mut FrontCtx, delta: i32) -> Option<Trigger> {
        if self.modal_up() {
            return None;
        }
        let t = self.inner.wheel(ctx, delta);
        self.route(t)
    }

    fn mouse(&mut self, ctx: &mut FrontCtx, down: bool, p: Point) -> Option<Trigger> {
        if self.modal_up() {
            return None;
        }
        let t = self.inner.mouse(ctx, down, p);
        self.route(t)
    }

    fn key(&mut self, ctx: &mut FrontCtx, k: u16) -> Option<Trigger> {
        if let Phase::DeletePassword(_) = self.phase {
            if k == crate::ui::front_end::control::vk::BACKSPACE {
                self.password.backspace();
            }
            return None;
        }
        if self.modal_up() {
            return None;
        }
        let t = self.inner.key(ctx, k);
        self.route(t)
    }

    fn key_up(&mut self, ctx: &mut FrontCtx, k: u16) {
        self.inner.key_up(ctx, k);
    }

    fn overlay(&mut self, now_ms: u64, adv: &dyn Fn(u16, &[u16]) -> i32) -> Vec<DrawItem> {
        self.inner.overlay(now_ms, adv)
    }

    fn sync(&mut self, controls: &mut [Control]) {
        self.inner.sync(controls);
        if let Some(c) = self.prompt_ix.and_then(|i| controls.get_mut(i)) {
            show_field(c, &self.password, true, self.now_ms, 15);
        }
    }

    fn ui_elements(&self) -> Vec<UiElement> {
        self.inner.ui_elements()
    }

    fn loads_sky_palette(&self) -> bool {
        self.inner.loads_sky_palette()
    }

    fn palette(&self) -> Option<[&'static str; 2]> {
        self.inner.palette()
    }
}

pub fn register(reg: &mut Registry, online: OnlineHandle) {
    reg.register(ONLINE_CHARS, Box::new(OnlineChars::new(online)));
}
