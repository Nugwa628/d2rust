//! The NPC shop panel (ui 0x0C).
//!
//! Draw order: the `buysell` art quads, [the
//! NPC store grid, drawn by the inventory grid code, not here], the action
//! buttons, the tabs (each tab: cel, then caption, as in
//! `panel-layout.tsv`).
//!
//! Not implemented yet:
//! - The action buttons' DC6 file is not known, so they are
//!   exposed as (frame, x, y) through [`ShopPanel::button_cels`] and not
//!   drawn. The 4 × 4 X table is indexed by `mode` = button count − 1 here
//!   (row `m` holds `m + 1` entries); how `mode` derives from the
//!   shop's button count is not settled.
//! - Hit rectangles of the tabs and action buttons, and what a
//!   click on them does.
//! - The buy 0x32 / sell 0x33 / repair 0x35 builders: the values the
//!   client writes in the fields the server does not read (client price u32
//!   @13, 0x35 u16 @9) and which click picks which transaction `t` are not
//!   known (the server side is). Gamble identify 0x37 and hire
//!   0x36: field layout and sender inputs not known here. Not built.
//! - The close of the shop sends 0x30 (the interaction's end);
//!   bytes 1–4 of that message (not read by the server)
//!   are written 0 here.
//! - The flag that hides the captions is an input
//!   ([`ShopPanel::captions_hidden`]); its meaning is not known.

use super::menu_box::{MenuBox, MenuError, MenuParams};
use super::{emit_static_draws, no_extra, text, PanelEnv, PanelOutput, PanelTables, TextMeasure};
use crate::ui::draw::UiDrawSink;
use crate::ui::layout::{PanelKey, Screen};
use crate::ui::messages::{msg_u32s, Metrics};
use crate::ui::panel::{ClientIntent, StringLookup};

/// The shop's ui id.
pub const UI_SHOP: u8 = 0x0C;
/// Font16 (`text-fonts.tsv` id 1), the tab caption font.
pub const FONT16: u16 = 1;
/// Action button X table: row `m` has `m + 1`
/// entries.
pub const BUTTON_X: [[i32; 4]; 4] = [
    [273, 0, 0, 0],
    [169, 273, 0, 0],
    [169, 221, 273, 0],
    [116, 169, 221, 273],
];

/// One tab record of the shop's tab table (stride 18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShopTab {
    /// i32 @0.
    pub x: i32,
    /// i32 @4.
    pub y: i32,
    /// String id u16 @8.
    pub string: u16,
    /// u32 @0x0A.
    pub active: bool,
    /// u32 @0x0E.
    pub visible: bool,
}

/// The default tab records: Armor 4036, Weapons 4037, Weapons
/// 4037, Misc 4039 at x 42, 121, 201, 281, y 20. Active / visible are set
/// at run time; the defaults here are both false.
pub const DEFAULT_TABS: [ShopTab; 4] = [
    ShopTab {
        x: 42,
        y: 20,
        string: 4036,
        active: false,
        visible: false,
    },
    ShopTab {
        x: 121,
        y: 20,
        string: 4037,
        active: false,
        visible: false,
    },
    ShopTab {
        x: 201,
        y: 20,
        string: 4037,
        active: false,
        visible: false,
    },
    ShopTab {
        x: 281,
        y: 20,
        string: 4039,
        active: false,
        visible: false,
    },
];

/// One button record of the shop's button table (stride 20).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonRec {
    pub enabled: bool,
    pub state: u32,
    pub string: u16,
    pub base_frame: u16,
}

/// The shop setup's buttons: count = `len`, mode 3 for the NPC classes that
/// have buttons.
pub fn shop_button_records(npc: u32) -> Vec<ButtonRec> {
    let b = |enabled, string, base_frame| ButtonRec {
        enabled,
        state: 0,
        string,
        base_frame,
    };
    match npc {
        147 | 148 | 177 | 199 | 202 | 252 | 254 | 255 | 405 | 512 | 513 => vec![
            b(true, 3335, 2),
            b(true, 3336, 4),
            b(false, 0, 0),
            b(true, 4144, 10),
        ],
        154 | 178 | 253 | 257 | 511 => vec![
            b(true, 3335, 2),
            b(true, 3336, 4),
            b(true, 3338, 6),
            b(true, 10095, 18),
        ],
        _ => Vec::new(),
    }
}

/// The start page of the shop: the page and whether
/// the 500 ms open delay is set.
pub fn shop_start_page(npc: u32) -> (u8, bool) {
    match npc {
        147 | 512 => (0, false),
        154 | 511 => (1, false),
        148 | 177 | 178 | 202 | 252 | 253 | 255 | 257 | 405 | 513 => (3, true),
        _ => (0, false),
    }
}

/// Shop tabs: `counts` are the store items per page
/// 0–4, `max_page` is the highest store page. Returns the tab records' (active,
/// visible) and the current page. Pages > 4 are read as 0.
pub fn shop_tabs(page: u8, counts: [u32; 5], max_page: u8) -> ([(bool, bool); 4], u8) {
    let mut cur = if page > 4 { 0 } else { page };
    let mut tabs = [(false, false); 4];
    for (i, t) in tabs.iter_mut().enumerate() {
        t.1 = counts[i] > 0;
    }
    for _ in 0..4 {
        if counts.get(cur as usize).copied().unwrap_or(0) > 0 {
            break;
        }
        cur = if cur == 0xFF || cur + 1 > max_page {
            0
        } else {
            cur + 1
        };
    }
    if let Some(t) = tabs.get_mut(cur as usize) {
        t.0 = true;
    }
    (tabs, cur)
}

/// One action button record (stride 20): state u16 @0,
/// base frame u16 @0x0A.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShopButton {
    pub state: u16,
    pub base: u16,
}

/// The shop panel's inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopPanel {
    pub tabs: [ShopTab; 4],
    /// The shown action buttons.
    pub buttons: Vec<ShopButton>,
    /// ≠ 0: tab captions are not drawn.
    pub captions_hidden: bool,
    /// GUID of the NPC traded with.
    pub npc_guid: u32,
}

impl ShopPanel {
    /// Draws the art quads, then the tabs. The store grid and the
    /// action buttons, drawn between them, are not drawn here: see
    /// [`Self::draw_art`] and [`Self::draw_tabs`].
    pub fn draw(
        &self,
        t: &PanelTables,
        env: &PanelEnv,
        strings: &dyn StringLookup,
        measure: &dyn TextMeasure,
        out: &mut dyn UiDrawSink,
    ) {
        self.draw_art(t, env, out);
        self.draw_tabs(t, env, strings, measure, out);
    }

    /// The art quads, the panel's first draws.
    pub fn draw_art(&self, t: &PanelTables, env: &PanelEnv, out: &mut dyn UiDrawSink) {
        let cond = env.cond(false, &no_extra);
        emit_static_draws(
            t,
            PanelKey::Ui(UI_SHOP),
            &cond,
            None,
            &|r| r.item.starts_with("art"),
            out,
        );
    }

    /// The tabs and their captions, after the store grid and the
    /// action buttons (`gen-ui-shop` tick 120).
    pub fn draw_tabs(
        &self,
        t: &PanelTables,
        env: &PanelEnv,
        strings: &dyn StringLookup,
        measure: &dyn TextMeasure,
        out: &mut dyn UiDrawSink,
    ) {
        let Some(tabs_file) = t.files.id("panel\\buyselltabs") else {
            return;
        };
        let s = &env.screen;
        for (i, tab) in (0i32..).zip(self.tabs.iter()) {
            if !tab.visible {
                continue;
            }
            let frame = if tab.active { i } else { i + 4 };
            out.push(super::cel(
                tabs_file,
                frame as u32,
                s.sx() + 80 * i,
                s.h + s.sy() - 449,
            ));
            if self.captions_hidden {
                continue;
            }
            let Some(str16) = strings.get_id(tab.string) else {
                continue;
            };
            let Some(width) = measure.width(FONT16, str16) else {
                continue;
            };
            let color = if tab.active { 4 } else { 0 };
            out.push(text(
                str16.to_vec(),
                s.sx() + tab.x - width / 2,
                s.h + s.sy() - 480 + tab.y,
                FONT16,
                color,
            ));
        }
    }

    /// The action buttons as (frame, x, y) draw positions: frame = base +
    /// state, x = `sx − 1 + X[n − 1][i]`, y = `H + sy − 63`.
    /// Empty for 0 or more than 4 buttons.
    pub fn button_cels(&self, s: &Screen) -> Vec<(u32, i32, i32)> {
        let n = self.buttons.len();
        if n == 0 || n > 4 {
            return Vec::new();
        }
        self.buttons
            .iter()
            .zip(BUTTON_X[n - 1])
            .map(|(b, x)| {
                (
                    u32::from(b.base) + u32::from(b.state),
                    s.sx() - 1 + x,
                    s.h + s.sy() - 63,
                )
            })
            .collect()
    }

    /// Closing the shop ends the interaction: C→S 0x30 `[u32 1][u32 G]`.
    pub fn close_intent(&self) -> Vec<PanelOutput> {
        vec![PanelOutput::Intent(ClientIntent(
            super::npc::msg_chat_end(self.npc_guid).to_vec(),
        ))]
    }
}

// ---------------------------------------------------------------------
// Shop transactions.
// ---------------------------------------------------------------------

/// A quick click within 500 ms of the last send is refused.
pub const QUICK_REFUSE_MS: u32 = 500;
/// Repair all within 2,000 ms of the previous one is refused.
pub const REPAIR_ALL_REFUSE_MS: u32 = 2000;
/// NPC classes of the repair-all branch and the repair button.
pub const REPAIR_CLASSES: [u32; 5] = [154, 178, 253, 257, 511];
/// NPC classes a player item sells to.
pub const SELL_CLASSES: [u32; 12] = [147, 148, 177, 199, 202, 252, 254, 255, 405, 512, 513, 514];
/// NPC classes that identify.
pub const IDENTIFY_CLASSES: [u32; 5] = [244, 245, 246, 265, 520];
/// `MK_SHIFT` in the window message wParam: `flags` bit 2.
pub const MK_SHIFT: u32 = 4;

/// The transaction kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxKind {
    Buy = 1,
    Sell = 2,
    Repair = 3,
    Identify = 4,
    Hire = 5,
}

/// The pending transaction: its kind with the item GUID
/// and class, the price and `t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub kind: TxKind,
    pub item_guid: u32,
    pub item_class: u32,
    pub price: u32,
    pub t: u8,
    /// The repair-all branch (item GUID 0).
    pub repair_all: bool,
}

/// What a click knows of its item.
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemFacts {
    pub guid: u32,
    pub class: u32,
    /// The item is at location 4.
    pub at_location_4: bool,
    /// ≠ 0.
    pub sellable: bool,
    pub repairable: bool,
    /// = 1.
    pub type_ok_16: bool,
}

/// The click arguments: a1 … a6 and the time.
#[derive(Clone, Copy, Debug)]
pub struct ClickArgs {
    /// The interaction's NPC class `c`; `None`: no interaction.
    pub npc_class: Option<u32>,
    pub npc_guid: u32,
    pub item: ItemFacts,
    /// a1: 1 = a player item, else a store item.
    pub a1: u8,
    /// a4: the window message wParam (bit 2 = `MK_SHIFT`).
    pub a4: u32,
    /// a5: the quick flag.
    pub quick: bool,
    /// a6 ≠ 0: repair all.
    pub repair_all: bool,
    /// `GetTickCount`.
    pub now: u32,
    /// The item facts of an immediate send, looked up by the caller.
    pub send_facts: SendFacts,
}

/// What the click reads of the world.
pub struct ClickEnv<'a> {
    /// The player has a cursor item.
    pub cursor_item: bool,
    /// The NPC runs a gamble shop.
    pub gamble_shop: bool,
    /// The repair-all button is on.
    pub repair_all_button_on: bool,
    /// The repair button is on.
    pub repair_button_on: bool,
    /// The price of the repair-all.
    pub repair_all_price: u32,
    pub price: &'a dyn Fn(u8) -> u32,
}

/// What a shop step asks of the rest of the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShopEffect {
    Send(ClientIntent),
    Sound(u32),
    /// The waiting note.
    WaitingNote,
    /// The confirm dialog.
    Confirm(TxKind),
    /// The pending transaction is cancelled.
    Cancel,
}

/// The result of a click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickOutcome {
    Refused,
    /// Sent at once or the confirm dialog opened.
    Effects(Vec<ShopEffect>),
}

/// The facts the send reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct SendFacts {
    /// The pending item exists.
    pub item_found: bool,
    /// The item's mode (unit +0x10, low 16 bits).
    pub item_mode: u16,
    /// The item's stat 72 (durability).
    pub durability: u32,
    /// The `items.txt` record byte +0x1A5 ≠ 0.
    pub item_flag_1a5: bool,
    /// The item is the player's cursor item (a sell).
    pub item_is_cursor: bool,
}

/// The shop transaction state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShopTx {
    pub pending: Option<Pending>,
    /// The NPC menu state: 4 confirm, 5 waiting, 10 hire.
    pub menu_state: u32,
    /// The time of the last transaction sent.
    pub last_send: u32,
    /// The time of the last repair-all.
    pub last_repair_all: u32,
    /// The item of the pending sell is the cursor item.
    pub sell_cursor: bool,
    /// The gamble shop flag at the click, for the 0x32 message.
    pub gamble: bool,
    pub npc_guid: u32,
}

impl ShopTx {
    /// The click.
    pub fn click(&mut self, a: &ClickArgs, env: &ClickEnv<'_>) -> ClickOutcome {
        let Some(c) = a.npc_class else {
            return ClickOutcome::Refused;
        };
        self.npc_guid = a.npc_guid;
        self.gamble = env.gamble_shop;
        if a.repair_all {
            // Only for the repair classes and only while the button is on:
            // kind 3, item GUID 0, sent at once.
            if !REPAIR_CLASSES.contains(&c) || !env.repair_all_button_on {
                return ClickOutcome::Refused;
            }
            self.pending = Some(Pending {
                kind: TxKind::Repair,
                item_guid: 0,
                item_class: 0,
                price: env.repair_all_price,
                t: 3,
                repair_all: true,
            });
            return ClickOutcome::Effects(self.send(a.a4, a.now, &a.send_facts));
        }
        if a.item.at_location_4 {
            return ClickOutcome::Refused;
        }
        if a.quick && a.now.wrapping_sub(self.last_send) < QUICK_REFUSE_MS {
            return ClickOutcome::Refused;
        }
        let (kind, t) = if a.a1 != 1 {
            // A store item: refused while the player has a cursor item.
            if env.cursor_item {
                return ClickOutcome::Refused;
            }
            (TxKind::Buy, if env.gamble_shop { 2 } else { 0 })
        } else if SELL_CLASSES.contains(&c) {
            (TxKind::Sell, 1)
        } else if REPAIR_CLASSES.contains(&c) {
            if env.repair_button_on {
                if !a.item.repairable {
                    return ClickOutcome::Refused;
                }
                (TxKind::Repair, 3)
            } else {
                (TxKind::Sell, 1)
            }
        } else if IDENTIFY_CLASSES.contains(&c) {
            if a.item.type_ok_16 {
                return ClickOutcome::Refused;
            }
            (TxKind::Identify, 0)
        } else {
            return ClickOutcome::Refused;
        };
        // A sell needs a sellable item.
        if kind == TxKind::Sell && !a.item.sellable {
            return ClickOutcome::Refused;
        }
        let price = if kind == TxKind::Identify {
            0
        } else {
            (env.price)(t)
        };
        self.pending = Some(Pending {
            kind,
            item_guid: a.item.guid,
            item_class: a.item.class,
            price,
            t,
            repair_all: false,
        });
        if matches!(kind, TxKind::Buy | TxKind::Sell) && a.quick {
            return ClickOutcome::Effects(self.send(a.a4, a.now, &a.send_facts));
        }
        // Every other case: click sound 1, menu state:= 4 and the confirm
        // dialog.
        self.menu_state = 4;
        ClickOutcome::Effects(vec![ShopEffect::Sound(1), ShopEffect::Confirm(kind)])
    }

    /// The pending transaction is cancelled.
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    /// Sends the pending transaction with `flags`. `facts` describe the pending item
    /// (looked up now).
    pub fn send_with(&mut self, flags: u32, now: u32, facts: &SendFacts) -> Vec<ShopEffect> {
        self.send(flags, now, facts)
    }

    fn send(&mut self, flags: u32, now: u32, facts: &SendFacts) -> Vec<ShopEffect> {
        let Some(p) = self.pending else {
            return vec![ShopEffect::Cancel];
        };
        let id = match p.kind {
            TxKind::Buy => 0x32,
            TxKind::Sell => 0x33,
            TxKind::Repair => 0x35,
            // Other kinds cancel.
            _ => {
                self.pending = None;
                return vec![ShopEffect::Cancel];
            }
        };
        let mut e = Vec::new();
        if p.kind == TxKind::Repair {
            e.push(ShopEffect::Sound(0x0F));
        }
        if p.repair_all {
            // A refused repair all only
            // cancels and keeps the pending transaction.
            if now.wrapping_sub(self.last_repair_all) < REPAIR_ALL_REFUSE_MS {
                return vec![ShopEffect::Cancel];
            }
        } else if !facts.item_found {
            // A missing item aborts: pending:= 0, no message.
            self.pending = None;
            return Vec::new();
        }
        let m = u32::from(facts.item_mode);
        let (a9, a13) = match (p.kind, p.repair_all) {
            (TxKind::Buy, _) => {
                let mut v = m << 16;
                if self.gamble {
                    v |= 2;
                }
                if flags & MK_SHIFT != 0 && facts.item_flag_1a5 {
                    v |= 0x8000_0000;
                }
                (v, p.price)
            }
            (TxKind::Sell, _) => (m, p.price),
            (TxKind::Repair, true) => (0, 0x8000_0000),
            (_, _) => (m, facts.durability),
        };
        e.push(ShopEffect::Send(msg_u32s(
            id,
            &[self.npc_guid, p.item_guid, a9, a13],
        )));
        self.menu_state = 5;
        self.last_send = now;
        if p.repair_all {
            self.last_repair_all = now;
        }
        if p.kind == TxKind::Sell {
            self.sell_cursor = facts.item_is_cursor;
        }
        e.push(ShopEffect::WaitingNote);
        e
    }
}

/// The handlers of the confirm dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmHandler {
    Yes,
    /// (also p1).
    No,
}

/// Strings of the confirm dialog.
pub const CONFIRM_CAPTIONS: [(TxKind, u16); 5] = [
    (TxKind::Buy, 3348),
    (TxKind::Sell, 3347),
    (TxKind::Repair, 3351),
    (TxKind::Identify, 3350),
    (TxKind::Hire, 3352),
];
pub const STR_GOLD: u16 = 3346;
pub const STR_YES: u16 = 3344;
pub const STR_NO: u16 = 3345;
/// The item name is cut at its first LF to 191 units.
pub const CONFIRM_NAME_MAX: usize = 191;

/// The confirm dialog, anchored at the current mouse
/// position: p1 = No, p4 = 1, p5 = 1, p9 = 1, style 1. `item_name` and
/// `price` are given for kinds 1–4 with the pending item found.
pub fn confirm_box(
    kind: TxKind,
    item_name: Option<(&[u16], u32)>,
    mouse: (i32, i32),
    strings: &dyn Fn(u16) -> Vec<u16>,
    frame: (i32, i32),
    m: &dyn Metrics,
) -> Result<MenuBox<ConfirmHandler>, MenuError> {
    let mut b = MenuBox::new(
        mouse,
        MenuParams {
            p1: Some(ConfirmHandler::No),
            p4: true,
            p5: true,
            p9: 1,
            ..Default::default()
        },
    )
    .expect("p1 is set");
    b.set_style(1);
    let cap = CONFIRM_CAPTIONS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|&(_, s)| s)
        .expect("every kind has a caption");
    b.add_item(&strings(cap), 21, 0, 4, 1, None, false, m)?;
    if kind != TxKind::Hire {
        if let Some((name, price)) = item_name {
            let name: Vec<u16> = name
                .iter()
                .copied()
                .take_while(|&u| u != 0x0A)
                .take(CONFIRM_NAME_MAX)
                .collect();
            b.add_item(&name, 15, 0, 4, 1, None, false, m)?;
            b.add_item(&strings(STR_GOLD), 15, 0, 4, 1, None, false, m)?;
            let num: Vec<u16> = price.to_string().encode_utf16().collect();
            b.add_item(&num, 15, 0, 4, 1, None, false, m)?;
        }
    }
    b.add_item(
        &strings(STR_YES),
        15,
        0,
        0,
        1,
        Some(ConfirmHandler::Yes),
        true,
        m,
    )?;
    b.add_item(
        &strings(STR_NO),
        15,
        0,
        0,
        1,
        Some(ConfirmHandler::No),
        true,
        m,
    )?;
    b.layout(frame.0, frame.1, m)?;
    Ok(b)
}

/// What an answer of the confirm dialog does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmResult {
    /// Yes, kind 5: close the box, send the hire (C→S 0x36).
    Hire,
    /// Yes, menu state 4: close the box, send the transaction (flags 0).
    Send,
    /// No, kind 5: the hire list again.
    HireList,
    /// No: close; cancel the pending transaction, menu
    /// state 3, the cursor press.
    Cancel,
    /// Yes with neither kind 5 nor menu state 4.
    Nothing,
}

/// The answer of the confirm dialog.
pub fn confirm_answer(kind: TxKind, menu_state: u32, yes: bool) -> ConfirmResult {
    match (yes, kind) {
        (true, TxKind::Hire) => ConfirmResult::Hire,
        (true, _) if menu_state == 4 => ConfirmResult::Send,
        (true, _) => ConfirmResult::Nothing,
        (false, TxKind::Hire) => ConfirmResult::HireList,
        (false, _) => ConfirmResult::Cancel,
    }
}

/// The callers of the click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// WM_RBUTTONDOWN on the store grid.
    StoreGridRightClick,
    /// WM_LBUTTONDOWN with a cursor item dropped on the store
    /// grid.
    DropOnStoreGrid,
    /// A grid click, a1 = the caller's.
    GridClick(u8),
    /// The body locations (worn items).
    BodyLocation,
    /// WM_LBUTTONUP on the button with base frame 18; the
    /// value is what the button's state was compared with (pressed).
    RepairAll(u8),
}

/// a1, a4, a5 (quick) and a6 of a caller; `wparam` is the window
/// message's.
pub fn caller_args(c: Caller, wparam: u32) -> (u8, u32, bool, bool) {
    match c {
        Caller::StoreGridRightClick => (0, wparam, true, false),
        Caller::DropOnStoreGrid => (1, wparam, true, false),
        Caller::GridClick(a1) => (a1, 0, false, false),
        Caller::BodyLocation => (1, 0, false, false),
        Caller::RepairAll(e) => (e, 0, e != 0, e != 0),
    }
}

#[cfg(test)]
mod tests_c2ui;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::{ImageRef, UiDraw};

    struct Strs;
    impl StringLookup for Strs {
        fn get(&self, _: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            const A: &[u16] = &[65, 114];
            const W: &[u16] = &[87, 101, 97];
            const M: &[u16] = &[77];
            match id {
                4036 => Some(A),
                4037 => Some(W),
                4039 => Some(M),
                _ => None,
            }
        }
    }
    /// Width = 10 × length (the sink measures for real).
    struct Meas;
    impl TextMeasure for Meas {
        fn width(&self, font: u16, t: &[u16]) -> Option<i32> {
            assert_eq!(font, FONT16);
            Some(10 * t.len() as i32 + 1)
        }
    }

    fn panel() -> ShopPanel {
        let mut tabs = DEFAULT_TABS;
        for t in &mut tabs {
            t.visible = true;
        }
        tabs[1].active = true;
        tabs[3].visible = false;
        ShopPanel {
            tabs,
            buttons: vec![],
            captions_hidden: false,
            npc_guid: 7,
        }
    }

    enum D {
        I(String, u32, i32, i32),
        T(Vec<u16>, i32, i32, u16, u16),
    }

    fn run(p: &ShopPanel, screen: Screen) -> Vec<D> {
        let t = PanelTables::load().unwrap();
        let env = PanelEnv {
            screen,
            open_mode: 3,
            exp: true,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        p.draw(&t, &env, &Strs, &Meas, &mut out);
        out.into_iter()
            .map(|d| match d {
                UiDraw::Image(i) => {
                    let ImageRef { file, frame } = i.image;
                    D::I(t.files.name(file).unwrap().into(), frame, i.at.x, i.at.y)
                }
                UiDraw::Text(x) => D::T(x.text, x.at.x, x.at.y, x.style.font, x.style.color),
                UiDraw::Rect(r) => panic!("rectangle {r:?}"),
                UiDraw::Line(_) => panic!("line"),
            })
            .collect()
    }

    // Partial: art quads and tabs; grid and buttons excluded.
    #[test]
    fn shop_art_and_tabs_800() {
        let d = run(&panel(), Screen::R800);
        let mut it = d.iter();
        for (f, x, y) in [(0, 80, 316), (1, 336, 316), (2, 80, 492), (3, 336, 492)] {
            match it.next().unwrap() {
                D::I(n, fr, xx, yy) => {
                    assert_eq!((n.as_str(), *fr, *xx, *yy), ("panel\\buysell", f, x, y))
                }
                D::T(..) => panic!(),
            }
        }
        let rest: Vec<_> = it.collect();
        assert_eq!(rest.len(), 6);
        // Tab 0 inactive: frame 4 at (80, 91); caption "Ar" width 21 at
        // x = 80 + 42 − 10, y = 600 − 60 − 480 + 20, color 0.
        match (rest[0], rest[1]) {
            (D::I(n, 4, 80, 91), D::T(s, 112, 80, 1, 0)) => {
                assert_eq!(n, "panel\\buyselltabs");
                assert_eq!(s, &vec![65, 114]);
            }
            _ => panic!(),
        }
        // Tab 1 active: frame 1 at (160, 91); "Wea" width 31, x 80 + 121 − 15.
        assert!(matches!(
            (rest[2], rest[3]),
            (D::I(_, 1, 160, 91), D::T(_, 186, 80, 1, 4))
        ));
        assert!(matches!(
            (rest[4], rest[5]),
            (D::I(_, 6, 240, 91), D::T(_, 266, 80, 1, 0))
        ));
    }

    // Partial: caption suppression flag.
    #[test]
    fn shop_captions_hidden() {
        let mut p = panel();
        p.captions_hidden = true;
        let d = run(&p, Screen::R640);
        assert_eq!(d.len(), 4 + 3);
        assert!(d.iter().all(|x| matches!(x, D::I(..))));
        assert!(matches!(d[4], D::I(_, 4, 0, 31)));
    }

    // Partial: button positions and frames; file unnamed.
    #[test]
    fn shop_button_positions() {
        let b = |base, state| ShopButton { state, base };
        let mut p = panel();
        p.buttons = vec![b(0, 1), b(2, 0), b(4, 1), b(6, 0)];
        assert_eq!(
            p.button_cels(&Screen::R640),
            vec![(1, 115, 417), (2, 168, 417), (5, 220, 417), (6, 272, 417)]
        );
        p.buttons = vec![b(8, 0)];
        assert_eq!(p.button_cels(&Screen::R800), vec![(8, 352, 477)]);
        p.buttons.clear();
        assert!(p.button_cels(&Screen::R800).is_empty());
    }

    // Partial: close of the shop only; the bytes are
    // the interaction end's 0x30 [1 u32][GUID u32].
    #[test]
    fn shop_close_sends_0x30() {
        let o = panel().close_intent();
        assert_eq!(
            o,
            vec![PanelOutput::Intent(ClientIntent(vec![
                0x30, 1, 0, 0, 0, 7, 0, 0, 0
            ]))]
        );
    }

    // ---- ----

    fn price_fn(t: u8) -> u32 {
        100 + u32::from(t)
    }

    fn click_env(price: &dyn Fn(u8) -> u32) -> ClickEnv<'_> {
        ClickEnv {
            cursor_item: false,
            gamble_shop: false,
            repair_all_button_on: false,
            repair_button_on: false,
            repair_all_price: 77,
            price,
        }
    }

    fn click_args(class: u32, a1: u8) -> ClickArgs {
        ClickArgs {
            npc_class: Some(class),
            npc_guid: 5,
            item: ItemFacts {
                guid: 9,
                class: 0x22,
                at_location_4: false,
                sellable: true,
                repairable: true,
                type_ok_16: false,
            },
            a1,
            a4: 0,
            quick: false,
            repair_all: false,
            now: 100_000,
            send_facts: SendFacts {
                item_found: true,
                item_mode: 0,
                durability: 41,
                item_flag_1a5: true,
                item_is_cursor: false,
            },
        }
    }

    fn sent(e: &ClickOutcome) -> Vec<Vec<u8>> {
        match e {
            ClickOutcome::Effects(v) => v
                .iter()
                .filter_map(|e| match e {
                    ShopEffect::Send(ClientIntent(b)) => Some(b.clone()),
                    _ => None,
                })
                .collect(),
            ClickOutcome::Refused => Vec::new(),
        }
    }

    fn found(mode: u16) -> SendFacts {
        SendFacts {
            item_found: true,
            item_mode: mode,
            durability: 41,
            item_flag_1a5: true,
            item_is_cursor: false,
        }
    }

    #[test]
    fn click_gates() {
        let mut t = ShopTx::default();
        let pf = price_fn;
        let env = click_env(&pf);
        // No interaction NPC: nothing.
        let mut a = click_args(147, 1);
        a.npc_class = None;
        assert_eq!(t.click(&a, &env), ClickOutcome::Refused);
        assert!(t.pending.is_none());
        // The item at location 4: refused.
        let mut a = click_args(147, 1);
        a.item.at_location_4 = true;
        assert_eq!(t.click(&a, &env), ClickOutcome::Refused);
        // The quick flag within 500 ms of the last send: refused; later
        // not.
        t.last_send = 99_600;
        let mut a = click_args(147, 1);
        a.quick = true;
        assert_eq!(t.click(&a, &env), ClickOutcome::Refused);
        t.last_send = 99_500;
        assert!(sent(&t.click(&a, &env)).len() == 1);
        // Without the quick flag the recent send does not matter.
        t.last_send = 99_900;
        a.quick = false;
        assert!(matches!(t.click(&a, &env), ClickOutcome::Effects(_)));
        // Repair-all branch (a6): only the repair classes, only while the
        // button is on; kind 3, item GUID 0, sent at once.
        let mut env = click_env(&pf);
        let mut a = click_args(154, 1);
        a.repair_all = true;
        let mut t = ShopTx::default();
        assert_eq!(t.click(&a, &env), ClickOutcome::Refused);
        env.repair_all_button_on = true;
        let o = t.click(&a, &env);
        assert_eq!(sent(&o), vec![msg_u32s(0x35, &[5, 0, 0, 0x8000_0000]).0]);
        let mut t2 = ShopTx::default();
        a.npc_class = Some(147);
        assert_eq!(t2.click(&a, &env), ClickOutcome::Refused);
        for c in [154, 178, 253, 257, 511] {
            a.npc_class = Some(c);
            assert!(!sent(&ShopTx::default().click(&a, &env)).is_empty(), "{c}");
        }
        // Repair all is refused within 2,000 ms of the previous one.
        a.npc_class = Some(154);
        let mut t = ShopTx::default();
        assert_eq!(sent(&t.click(&a, &env)).len(), 1);
        a.now += 1999;
        assert!(sent(&t.click(&a, &env)).is_empty());
        a.now += 1;
        assert_eq!(sent(&t.click(&a, &env)).len(), 1);
        // A refused repair all keeps the pending transaction.
        let mut t = ShopTx::default();
        a.now = 10_000;
        assert_eq!(sent(&t.click(&a, &env)).len(), 1);
        let kept = t.pending;
        assert!(kept.is_some_and(|p| p.repair_all));
        let e = t.send_with(0, 11_000, &found(3));
        assert_eq!(e, vec![ShopEffect::Cancel]);
        assert_eq!(t.pending, kept);
        let e = t.send_with(0, 12_000, &found(3));
        assert!(
            matches!(e.last(), Some(ShopEffect::Send(_)))
                || e.iter().any(|x| matches!(x, ShopEffect::Send(_)))
        );
    }

    #[test]
    fn kind_and_price() {
        let pf = price_fn;
        let mut env = click_env(&pf);
        let kind = |t: &ShopTx| t.pending.map(|p| (p.kind, p.t, p.price));
        // A store item (a1 ≠ 1): refused while the player has a cursor
        // item; else kind 1 with t = 2 in a gamble shop, else 0.
        let mut t = ShopTx::default();
        env.cursor_item = true;
        assert_eq!(t.click(&click_args(147, 0), &env), ClickOutcome::Refused);
        env.cursor_item = false;
        t.click(&click_args(147, 0), &env);
        assert_eq!(kind(&t), Some((TxKind::Buy, 0, 100)));
        env.gamble_shop = true;
        t.click(&click_args(147, 0), &env);
        assert_eq!(kind(&t), Some((TxKind::Buy, 2, 102)));
        env.gamble_shop = false;
        // A player item: sell classes → t = 1, needs a sellable item.
        for c in [147, 148, 177, 199, 202, 252, 254, 255, 405, 512, 513, 514] {
            let mut t = ShopTx::default();
            t.click(&click_args(c, 1), &env);
            assert_eq!(kind(&t), Some((TxKind::Sell, 1, 101)), "class {c}");
        }
        let mut t = ShopTx::default();
        let mut a = click_args(147, 1);
        a.item.sellable = false;
        assert_eq!(t.click(&a, &env), ClickOutcome::Refused);
        // Repair classes with the repair button on: repair (t = 3) if the
        // item is repairable, else refused; off: sell.
        env.repair_button_on = true;
        for c in [154, 178, 253, 257, 511] {
            let mut t = ShopTx::default();
            t.click(&click_args(c, 1), &env);
            assert_eq!(kind(&t), Some((TxKind::Repair, 3, 103)), "class {c}");
        }
        let mut a = click_args(154, 1);
        a.item.repairable = false;
        assert_eq!(ShopTx::default().click(&a, &env), ClickOutcome::Refused);
        env.repair_button_on = false;
        let mut t = ShopTx::default();
        t.click(&click_args(154, 1), &env);
        assert_eq!(kind(&t), Some((TxKind::Sell, 1, 101)));
        // Cain's classes: kind 4 (no price) unless the item class check returns 1.
        for c in [244, 245, 246, 265, 520] {
            let mut t = ShopTx::default();
            t.click(&click_args(c, 1), &env);
            assert_eq!(kind(&t), Some((TxKind::Identify, 0, 0)), "class {c}");
        }
        let mut a = click_args(244, 1);
        a.item.type_ok_16 = true;
        assert_eq!(ShopTx::default().click(&a, &env), ClickOutcome::Refused);
        // Any other class: refused.
        assert_eq!(
            ShopTx::default().click(&click_args(150, 1), &env),
            ClickOutcome::Refused
        );
        // The pending fields: item GUID and class.
        let mut t = ShopTx::default();
        t.click(&click_args(147, 1), &env);
        let p = t.pending.unwrap();
        assert_eq!((p.item_guid, p.item_class, p.repair_all), (9, 0x22, false));
        // Kind 1 or 2 with the quick flag: sent at once; every other case:
        // click sound 1, menu state:= 4, the confirm dialog.
        let mut a = click_args(147, 0);
        a.quick = true;
        let mut t = ShopTx::default();
        assert_eq!(sent(&t.click(&a, &env)).len(), 1);
        assert_eq!(t.menu_state, 5);
        let mut t = ShopTx::default();
        assert_eq!(
            t.click(&click_args(147, 0), &env),
            ClickOutcome::Effects(vec![ShopEffect::Sound(1), ShopEffect::Confirm(TxKind::Buy)])
        );
        assert_eq!(t.menu_state, 4);
        // A quick click on a repair or identify still confirms.
        env.repair_button_on = true;
        let mut a = click_args(154, 1);
        a.quick = true;
        let mut t = ShopTx::default();
        assert_eq!(
            t.click(&a, &env),
            ClickOutcome::Effects(vec![
                ShopEffect::Sound(1),
                ShopEffect::Confirm(TxKind::Repair)
            ])
        );
    }

    // Test vector "sell: NPC GUID 5, item GUID 9, mode 0, price 35".
    #[test]
    fn send_messages() {
        let pf = |_t: u8| 35;
        let env = click_env(&pf);
        let mut t = ShopTx::default();
        t.click(&click_args(147, 1), &env);
        let e = t.send_with(0, 5000, &found(0));
        assert_eq!(
            e[0],
            ShopEffect::Send(ClientIntent(vec![
                0x33, 5, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0x23, 0, 0, 0
            ]))
        );
        // Then menu state:= 5, the sent time, the waiting note; a sell
        // records whether the item is the cursor item.
        assert_eq!(e[1], ShopEffect::WaitingNote);
        assert_eq!((t.menu_state, t.last_send), (5, 5000));
        let mut f = found(0);
        f.item_is_cursor = true;
        t.click(&click_args(147, 1), &env);
        t.send_with(0, 6000, &f);
        assert!(t.sell_cursor);
        // Sell: u32 @9 is the item's mode m.
        t.click(&click_args(147, 1), &env);
        let e = t.send_with(0, 7000, &found(3));
        assert_eq!(e[0], ShopEffect::Send(msg_u32s(0x33, &[5, 9, 3, 35])));
        // Buy: m << 16, OR 2 in a gamble shop, OR 0x80000000 with flags
        // bit 2 and the items.txt byte +0x1A5 ≠ 0.
        let mut env = click_env(&pf);
        let mut t = ShopTx::default();
        t.click(&click_args(147, 0), &env);
        assert_eq!(
            t.send_with(0, 1, &found(3))[0],
            ShopEffect::Send(msg_u32s(0x32, &[5, 9, 3 << 16, 35]))
        );
        t.click(&click_args(147, 0), &env);
        assert_eq!(
            t.send_with(MK_SHIFT, 2, &found(3))[0],
            ShopEffect::Send(msg_u32s(0x32, &[5, 9, (3 << 16) | 0x8000_0000, 35]))
        );
        let mut f = found(3);
        f.item_flag_1a5 = false;
        t.click(&click_args(147, 0), &env);
        assert_eq!(
            t.send_with(MK_SHIFT, 3, &f)[0],
            ShopEffect::Send(msg_u32s(0x32, &[5, 9, 3 << 16, 35]))
        );
        env.gamble_shop = true;
        t.click(&click_args(147, 0), &env);
        assert_eq!(
            t.send_with(0, 4, &found(3))[0],
            ShopEffect::Send(msg_u32s(0x32, &[5, 9, (3 << 16) | 2, 35]))
        );
        // Repair of one item: click sound 0x0F; u32 @13 is the item's
        // stat 72 (durability), not a price.
        let mut env = click_env(&pf);
        env.repair_button_on = true;
        let mut t = ShopTx::default();
        t.click(&click_args(154, 1), &env);
        let e = t.send_with(0, 10, &found(3));
        assert_eq!(e[0], ShopEffect::Sound(0x0F));
        assert_eq!(e[1], ShopEffect::Send(msg_u32s(0x35, &[5, 9, 3, 41])));
        // A missing item aborts: pending:= 0, no message.
        t.click(&click_args(154, 1), &env);
        assert!(t.send_with(0, 11, &SendFacts::default()).is_empty());
        assert!(t.pending.is_none());
        // Other kinds (and no pending transaction) cancel.
        let mut t = ShopTx::default();
        assert_eq!(t.send_with(0, 1, &found(0)), vec![ShopEffect::Cancel]);
        let mut t = ShopTx::default();
        t.pending = Some(Pending {
            kind: TxKind::Identify,
            item_guid: 9,
            item_class: 0,
            price: 0,
            t: 0,
            repair_all: false,
        });
        assert_eq!(t.send_with(0, 1, &found(0)), vec![ShopEffect::Cancel]);
        assert!(t.pending.is_none());
    }

    #[test]
    fn confirm_dialog() {
        let strings = |id: u16| -> Vec<u16> { format!("s{id}").encode_utf16().collect() };
        let name: Vec<u16> = "Short Sword\nDamage".encode_utf16().collect();
        let m = crate::ui::messages::testutil::Fixed;
        let texts = |b: &MenuBox<ConfirmHandler>| -> Vec<String> {
            b.items
                .iter()
                .map(|i| String::from_utf16(&i.text).unwrap())
                .collect()
        };
        // Buy: the caption, the item name up to its LF, "Gold:", the
        // price, Yes, No.
        let b = confirm_box(
            TxKind::Buy,
            Some((&name[..], 350)),
            (300, 200),
            &strings,
            (800, 600),
            &m,
        )
        .unwrap();
        assert_eq!(
            texts(&b),
            ["s3348", "Short Sword", "s3346", "350", "s3344", "s3345"]
        );
        assert_eq!(
            (b.style, b.params.p4, b.params.p5, b.params.p9),
            (1, true, true, 1)
        );
        assert_eq!(b.params.p1, Some(ConfirmHandler::No));
        assert_eq!(b.anchor, (300, 200));
        // Heights 21, 15, 15, 15; colors 4 (info) and 0 (answers); the
        // answers are selectable.
        assert_eq!(
            b.items
                .iter()
                .map(|i| (i.height, i.color, i.selectable))
                .collect::<Vec<_>>(),
            vec![
                (21, 4, false),
                (15, 4, false),
                (15, 4, false),
                (15, 4, false),
                (15, 0, true),
                (15, 0, true)
            ]
        );
        assert_eq!(b.items[4].handler, Some(ConfirmHandler::Yes));
        assert_eq!(b.items[5].handler, Some(ConfirmHandler::No));
        // The captions by kind; kind 5 has only the caption and the answers.
        for (k, s) in [
            (TxKind::Sell, "s3347"),
            (TxKind::Repair, "s3351"),
            (TxKind::Identify, "s3350"),
        ] {
            let b = confirm_box(k, Some((&name[..], 1)), (0, 0), &strings, (800, 600), &m).unwrap();
            assert_eq!(texts(&b)[0], s);
        }
        let b = confirm_box(TxKind::Hire, None, (0, 0), &strings, (800, 600), &m).unwrap();
        assert_eq!(texts(&b), ["s3352", "s3344", "s3345"]);
        // A kind without the item found shows the answers only.
        let b = confirm_box(TxKind::Buy, None, (0, 0), &strings, (800, 600), &m).unwrap();
        assert_eq!(texts(&b), ["s3348", "s3344", "s3345"]);
        // The item name is cut to 191 units.
        let long = vec![65u16; 300];
        let b = confirm_box(
            TxKind::Buy,
            Some((&long[..], 1)),
            (0, 0),
            &strings,
            (1200, 900),
            &m,
        )
        .unwrap();
        assert_eq!(b.items[1].text.len(), 119);
        // Yes: kind 5 → hire; menu state 4 → send with flags 0. No: kind 5
        // → the hire list again; else cancel.
        assert_eq!(confirm_answer(TxKind::Hire, 4, true), ConfirmResult::Hire);
        assert_eq!(confirm_answer(TxKind::Buy, 4, true), ConfirmResult::Send);
        assert_eq!(confirm_answer(TxKind::Buy, 5, true), ConfirmResult::Nothing);
        assert_eq!(
            confirm_answer(TxKind::Hire, 4, false),
            ConfirmResult::HireList
        );
        assert_eq!(
            confirm_answer(TxKind::Sell, 4, false),
            ConfirmResult::Cancel
        );
    }

    #[test]
    fn callers_of_the_click() {
        // Right click on the store grid: a1 0, a4 = wParam, quick, a6 0.
        assert_eq!(
            caller_args(Caller::StoreGridRightClick, MK_SHIFT),
            (0, MK_SHIFT, true, false)
        );
        // A cursor item dropped on the store grid: sell by drop.
        assert_eq!(
            caller_args(Caller::DropOnStoreGrid, MK_SHIFT),
            (1, MK_SHIFT, true, false)
        );
        // Grid click: a1 the caller's, no flags, no quick.
        assert_eq!(caller_args(Caller::GridClick(0), 7), (0, 0, false, false));
        assert_eq!(caller_args(Caller::GridClick(1), 7), (1, 0, false, false));
        // Body locations.
        assert_eq!(caller_args(Caller::BodyLocation, 7), (1, 0, false, false));
        // Repair all: a1, a5, a6 are the ECX value (pressed).
        assert_eq!(caller_args(Caller::RepairAll(1), 7), (1, 0, true, true));
        // So the 0x32 bit 31 needs a quick right-click or drop with Shift
        // held: left clicks go through the confirm dialog with flags 0.
        let pf = |_t: u8| 10;
        let env = click_env(&pf);
        let at9 = |b: &[u8]| u32::from_le_bytes([b[9], b[10], b[11], b[12]]);
        let (a1, a4, quick, a6) = caller_args(Caller::StoreGridRightClick, MK_SHIFT);
        let mut a = click_args(147, a1);
        (a.a4, a.quick, a.repair_all) = (a4, quick, a6);
        let mut t = ShopTx::default();
        let b = sent(&t.click(&a, &env)).remove(0);
        assert_eq!(at9(&b), 0x8000_0000);
        // The same right click without Shift: no bit.
        let (a1, a4, quick, a6) = caller_args(Caller::StoreGridRightClick, 0);
        let mut a = click_args(147, a1);
        (a.a4, a.quick, a.repair_all) = (a4, quick, a6);
        let b = sent(&ShopTx::default().click(&a, &env)).remove(0);
        assert_eq!(at9(&b), 0);
        // A left click: the confirm dialog, then the send with flags 0.
        let (a1, a4, quick, a6) = caller_args(Caller::GridClick(0), MK_SHIFT);
        let mut a = click_args(147, a1);
        (a.a4, a.quick, a.repair_all) = (a4, quick, a6);
        let mut t = ShopTx::default();
        assert!(sent(&t.click(&a, &env)).is_empty());
        assert_eq!(t.menu_state, 4);
        let e = t.send_with(0, 1, &found(0));
        let ShopEffect::Send(ClientIntent(b)) = &e[0] else {
            panic!()
        };
        assert_eq!(at9(b), 0);
    }
}
