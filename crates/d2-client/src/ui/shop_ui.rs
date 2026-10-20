//! The NPC shop (ui 0x0C) installed in the panel UI: the store the server
//! showed (S→C 0x9C action 0x0B, one per item) drawn on the shop's art with the
//! tabs and the action buttons, and the clicks that leave as C→S 0x32 buy, 0x33
//! sell and 0x35 repair all ([`ShopTx`]). The client decides nothing: the
//! server checks the trade and answers with S→C 0x2A and the item and gold
//! messages.
//!
//! The shop opens when the NPC menu's Trade / Gamble choice is made (the next
//! poll calls [`OriginalUi::open_shop`]) or, D2RUST's own, when a trade's store
//! items arrive for the trader nearest to the local player. It is the full-slot
//! ui 0x0C: ui 1 stays off and the root draws the inventory family under it; ui
//! 8 stays on. Its close is the interaction's end (C→S
//! 0x30, `SetUIState(8, off)`).
//!
//! Clicks go through the transaction rules with the shop's callers
//! ([`caller_args`]): a right click on a store item is the quick buy
//! (Shift = `MK_SHIFT`), a left click goes through the confirm dialog
//! ([`confirm_box`], drawn and answered by the NPC menu panel), a cursor item
//! dropped on the store grid is the quick sell; with the repair button down a
//! click on one of the player's items is the confirmed repair, whose 0x35
//! carries the item's stat 72. Action buttons stay down until clicked again;
//! repair all acts on the release over its pressed button.
//!
//! D2RUST's own, unverified (preview fills):
//! - the store grid (10 × 10 cells of 29 px) at (`sx` + 15, `H` + `sy` −
//!   400) with the store items packed row by row (the server's cells and the
//!   NPC's `inventory.bin` record are q-fix-seam-store-grid);
//! - the buy price shown is published by the server host
//!   ([`ShopPrices`]); the sell and repair prices are not computed
//!   (the client's sell and repair cost is open);
//! - a click on one of the player's items is a shop click only with the
//!   repair button down (when the player grid hands clicks on is not
//!   known); the sellable and repairable tests and the `items.txt` byte
//!   +0x1A5 are not read (the server checks);
//! - Close acts on the press (the release is not settled);
//! - the time of the transaction rules is the bridge frame × 40 ms (+ 60 s).

use std::cell::RefCell;
use std::rc::Rc;

use super::{OriginalUi, SharedRef};
use crate::bridge::items::{self, ItemView};
use crate::bridge::world::{ClientWorld, MONSTER};
use crate::ui::draw::UiDrawSink;
use crate::ui::geom::{Point, Rect};
use crate::ui::inv_grid::GridRecord;
use crate::ui::panel::StringLookup;
use crate::ui::panel::{ClientIntent, Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use crate::ui::panels::menu_box::MenuBox;
use crate::ui::panels::npc::msg_chat_end;
use crate::ui::panels::shop::{
    caller_args, confirm_box, shop_button_records, shop_start_page, shop_tabs, Caller, ClickArgs,
    ClickEnv, ClickOutcome, ConfirmHandler, ItemFacts, SendFacts, ShopButton, ShopEffect,
    ShopPanel, ShopTx, TxKind, DEFAULT_TABS, IDENTIFY_CLASSES, MK_SHIFT, REPAIR_CLASSES,
    SELL_CLASSES, UI_SHOP,
};
use crate::ui::panels::{cel, text, utf16, PanelOutput, TextMeasure};
use crate::ui::root::UiRoot;
use crate::ui::states::id;
use crate::ui::PointerButton;

pub use d2_server::rest::{ShopPrices, REPAIR_ALL_KEY};

/// The trade the shop panel is up for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShopOpen {
    pub npc_guid: u32,
    pub npc_class: u32,
}

/// The shop panel's state.
#[derive(Debug, Default)]
pub struct ShopState {
    pub open: Option<ShopOpen>,
    pub(super) tx: ShopTx,
    /// The current store page.
    page: u8,
    /// `ClientWorld::store_serial` seen by the last poll.
    serial: u32,
    /// Store records with a smaller `store_seq` belong to a closed trade.
    floor: u32,
    /// The pressed action button (state 1), by index.
    pressed: Option<usize>,
    prices: ShopPrices,
    /// The window is a gamble window (the menu chose Gamble).
    gamble: bool,
    /// The repair button is down: the next click on one of the player's items
    /// repairs it (D2RUST's own; unverified).
    repair_mode: bool,
    /// A shop closed since the last Trade / Gamble choice: a late store
    /// record (a sold item's copy) opens nothing until the next choice.
    closed: bool,
    /// The confirm dialog, drawn and
    /// answered by the NPC menu panel (ui 8 is open under the shop).
    pub(super) confirm: Option<ConfirmUp>,
    /// A send asked for the waiting note: opened by the
    /// next [`OriginalUi::npc_menu_poll`].
    pub(super) note: bool,
}

/// The open confirm dialog: the box, its kind and the send facts of the
/// pending item (looked up at the click).
#[derive(Clone, Debug)]
pub struct ConfirmUp {
    pub bx: MenuBox<ConfirmHandler>,
    pub kind: TxKind,
    pub facts: SendFacts,
    pub now: u32,
    /// The hire of a kind-5 dialog: (NPC GUID, record name).
    pub hire: Option<(u32, u16)>,
    /// The item index the left press went down on.
    pub pressed: Option<usize>,
}

impl ShopState {
    /// The single-item repair button is down.
    pub fn repair_mode(&self) -> bool {
        self.repair_mode
    }

    /// The window is a gamble window.
    pub fn gamble(&self) -> bool {
        self.gamble
    }

    /// The price the host published for an item.
    pub fn price(&self, guid: u32) -> Option<u32> {
        self.prices.get(guid)
    }

    /// The confirm dialog's kind and its items' text, when one is up.
    pub fn confirm(&self) -> Option<(TxKind, Vec<Vec<u16>>)> {
        let c = self.confirm.as_ref()?;
        Some((c.kind, c.bx.items.iter().map(|i| i.text.clone()).collect()))
    }

    /// The pressed action button, by index (buttons stay down).
    pub fn pressed(&self) -> Option<usize> {
        self.pressed
    }
}

pub type SharedShop = Rc<RefCell<ShopState>>;

/// Cells of the store grid, columns × rows.
const GRID: (u8, u8) = (10, 10);
const CELL: u8 = 29;
/// The action button hit width and height.
const BUTTON_W: i32 = 45;

/// The classes that trade (shop button table, repair, identify and sell
/// classes).
fn trades(class: u32) -> bool {
    !shop_button_records(class).is_empty()
        || REPAIR_CLASSES.contains(&class)
        || SELL_CLASSES.contains(&class)
        || IDENTIFY_CLASSES.contains(&class)
}

/// The store grid's `inventory.bin` record: an NPC is a monster owner, record 5
/// "Monster" (10 × 10), +16 at 800 × 600.
pub fn store_record(screen: &crate::ui::layout::Screen) -> usize {
    if screen.res2() {
        5 + 16
    } else {
        5
    }
}

/// Without `inventory.bin` rows: the `inventory.txt` "Monster" grid
/// (16, 305, 63, 352; right and bottom are the last pixel, one short of
/// 10 × 29), +(80, 60) at 800 × 600 as the game places them
/// (column 9 of row 0 at (357, 123)).
fn fallback_store_grid(screen: &crate::ui::layout::Screen) -> GridRecord {
    let (dx, dy) = (screen.sx(), -screen.sy() + screen.panel_dy());
    let (left, top) = (16 + dx, 63 + dy);
    GridRecord {
        grid_x: GRID.0,
        grid_y: GRID.1,
        left,
        right: left + i32::from(GRID.0) * i32::from(CELL) - 1,
        top,
        bottom: top + i32::from(GRID.1) * i32::from(CELL) - 1,
        cell_w: CELL,
        cell_h: CELL,
    }
}

pub(super) fn grid_record(sh: &super::Shared) -> GridRecord {
    let s = sh.config.screen;
    sh.items
        .layouts
        .as_ref()
        .and_then(|l| l.get(store_record(&s)))
        .map_or_else(|| fallback_store_grid(&s), |l| l.grid)
}

/// The hover handler over the open shop's store page, as
/// the shop's move handler calls it: no
/// cursor item, the mouse in the store grid. Sets the hovered store item
/// read by the store grid's tints. Not modelled: the shop mode
/// mode = 0 gate and the `inv` rectangle test (the grid lies
/// inside it).
pub(super) fn track_store_hover(
    sh: &super::Shared,
    world: &ClientWorld,
    st: &ShopState,
    at: Point,
) {
    if st.open.is_none() || items::cursor_item(world).is_some() {
        return;
    }
    let (_, page) = tabs_of(world, st);
    let list = page_items(sh, world, st, page);
    sh.items
        .track_hover_over(world, &sh.tables.files, &grid_record(sh), &list, at);
}

/// The footprint of a store item in cells (the art row's, else 1 × 1).
fn footprint(sh: &super::Shared, it: &ItemView) -> (i32, i32) {
    it.code
        .and_then(|c| sh.items.art.get(c))
        .map_or((1, 1), |r| {
            (i32::from(r.inv_w.max(1)), i32::from(r.inv_h.max(1)))
        })
}

/// The store items of the open trade on `page`, at the cells the stream
/// gave them: the server placed them in the
/// NPC's grid and the client draws and hit-tests
/// them there, without repacking. An item whose footprint leaves the
/// grid is not drawn or hit.
fn page_items(sh: &super::Shared, world: &ClientWorld, st: &ShopState, page: u8) -> Vec<ItemView> {
    let mut list: Vec<ItemView> = items::store_items(world)
        .into_iter()
        .filter(|i| i.store_seq > st.floor && i.page == page)
        .collect();
    list.sort_by_key(|i| i.store_seq);
    in_grid(list, |i| footprint(sh, i))
}

/// The items whose footprint (`size`) lies inside the store grid, left at
/// their stream cells.
fn in_grid(mut list: Vec<ItemView>, size: impl Fn(&ItemView) -> (i32, i32)) -> Vec<ItemView> {
    list.retain(|i| {
        let (w, h) = size(i);
        i32::from(i.x) + w <= i32::from(GRID.0) && i32::from(i.y) + h <= i32::from(GRID.1)
    });
    list
}

/// The store item whose footprint holds `at`.
fn item_at(sh: &super::Shared, g: &GridRecord, list: &[ItemView], at: Point) -> Option<ItemView> {
    if !g.contains_mouse(at) {
        return None;
    }
    let (c, r) = g.mouse_cell(at);
    let (c, r) = (c as i32, r as i32);
    list.iter()
        .find(|i| {
            let (w, h) = footprint(sh, i);
            let (x, y) = (i32::from(i.x), i32::from(i.y));
            (x..x + w).contains(&c) && (y..y + h).contains(&r)
        })
        .cloned()
}

/// The action button under `at` (`sx + X[n − 1][i]` < x < that + 45, `H + sy −
/// 109` < y < `H + sy − 65`, strict).
fn button_at(s: &crate::ui::layout::Screen, n: usize, at: Point) -> Option<usize> {
    let (lo, hi) = (s.h + s.sy() - 109, s.h + s.sy() - 65);
    if n == 0 || n > 4 || at.y <= lo || at.y >= hi {
        return None;
    }
    let xs = crate::ui::panels::shop::BUTTON_X[n - 1];
    (0..n).find(|&i| at.x > s.sx() + xs[i] && at.x < s.sx() + xs[i] + BUTTON_W)
}

/// Exact integer bands used by the tab pointer handler.
fn tab_rect(s: &crate::ui::layout::Screen, i: usize) -> Rect {
    let x = s.sx() + i as i32 * 80 + i32::from(i > 0);
    let w = match i {
        0 => 81,
        3 => 79,
        _ => 80,
    };
    Rect::new(x, s.h + s.sy() - 479, w, 31)
}

/// The tab records for the page counts.
fn tabs_of(world: &ClientWorld, st: &ShopState) -> ([(bool, bool); 4], u8) {
    let mut counts = [0u32; 5];
    for i in items::store_items(world)
        .iter()
        .filter(|i| i.store_seq > st.floor)
    {
        if let Some(c) = counts.get_mut(usize::from(i.page)) {
            *c += 1;
        }
    }
    shop_tabs(st.page, counts, 3)
}

struct NoMeasure;

impl TextMeasure for NoMeasure {
    fn width(&self, _: u16, _: &[u16]) -> Option<i32> {
        None
    }
}

/// The shop adapter (ui 0x0C, left slot).
pub struct ShopUi {
    pub(super) sh: SharedRef,
    pub(super) st: SharedShop,
}

impl ShopUi {
    fn panel(&self, world: &ClientWorld, st: &ShopState, open: ShopOpen) -> (ShopPanel, u8) {
        let (tabs, page) = tabs_of(world, st);
        let mut t = DEFAULT_TABS;
        for (rec, (active, visible)) in t.iter_mut().zip(tabs) {
            rec.active = active;
            rec.visible = visible;
        }
        let buttons = shop_button_records(open.npc_class)
            .iter()
            .enumerate()
            .map(|(i, b)| ShopButton {
                state: u16::from(st.pressed == Some(i)),
                base: b.base_frame,
            })
            .collect();
        (
            ShopPanel {
                tabs: t,
                buttons,
                captions_hidden: false,
                npc_guid: open.npc_guid,
            },
            page,
        )
    }

    /// The effects of a transaction step: the sends leave through the
    /// shared outputs, the confirm dialog opens at the mouse, the
    /// waiting note is asked of the NPC menu.
    fn apply(
        sh: &mut super::Shared,
        st: &mut ShopState,
        world: &ClientWorld,
        strings: &dyn StringLookup,
        facts: (SendFacts, u32, Option<&ItemView>),
        effects: Vec<ShopEffect>,
    ) {
        for e in effects {
            match e {
                ShopEffect::Send(i) => sh.outputs.push(PanelOutput::Intent(i)),
                ShopEffect::Sound(id) => sh.outputs.push(PanelOutput::Sound(id as i32)),
                ShopEffect::WaitingNote => st.note = true,
                ShopEffect::Confirm(kind) => {
                    let (send, now, item) = facts;
                    let name = item.and_then(|it| item_name(sh, world, it));
                    let price = st.tx.pending.map_or(0, |p| p.price);
                    let s = |id: u16| strings.get_id(id).map(<[u16]>::to_vec).unwrap_or_default();
                    let m = super::game_messages::Measure(sh.fonts.as_ref());
                    let frame = (sh.config.screen.w, sh.config.screen.h);
                    let mouse = (sh.mouse.x, sh.mouse.y);
                    let named = name.as_deref().map(|n| (n, price));
                    if let Ok(bx) = confirm_box(kind, named, mouse, &s, frame, &m) {
                        st.confirm = Some(ConfirmUp {
                            bx,
                            kind,
                            facts: send,
                            now,
                            hire: None,
                            pressed: None,
                        });
                    }
                }
                ShopEffect::Cancel => {}
            }
        }
    }

    /// The click of `caller` on
    /// `item` (`player`: one of the player's items). `shift` is the
    /// window message's `MK_SHIFT`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn click(
        sh: &mut super::Shared,
        st: &mut ShopState,
        open: ShopOpen,
        world: &ClientWorld,
        strings: &dyn StringLookup,
        caller: Caller,
        item: Option<&ItemView>,
        shift: bool,
    ) {
        let (a1, a4, quick, repair_all) = caller_args(caller, if shift { MK_SHIFT } else { 0 });
        let (guid, class, mode) = item.map_or((0, 0, 0), |it| {
            (
                it.key.guid,
                it.code.map_or(0, u32::from_le_bytes),
                u16::from(it.mode),
            )
        });
        // The item's stat 72 (durability) for a one-item repair.
        let durability = item.map_or(0, |it| world.base(it.key, 72, 0).max(0) as u32);
        // The UI clock ([`super::ui_clock`]), so the quick-send and
        // repair-all refuse gates last the same at any frame rate.
        let now = sh.ui_ms32(world.frames).wrapping_add(60_000);
        let send = SendFacts {
            item_found: item.is_some() || repair_all,
            item_mode: mode,
            durability,
            // D2RUST's own: the `items.txt` byte +0x1A5 is not in the client
            // tables yet; the server reads the bit.
            item_flag_1a5: false,
            item_is_cursor: item.is_some_and(|it| it.mode == items::mode::CURSOR),
        };
        let args = ClickArgs {
            npc_class: Some(open.npc_class),
            npc_guid: open.npc_guid,
            item: ItemFacts {
                guid,
                class,
                at_location_4: false,
                // D2RUST's own: the sellable and repairable tests are not
                // modelled; the server checks the sell and the repair.
                sellable: true,
                repairable: true,
                type_ok_16: false,
            },
            a1,
            a4,
            quick,
            repair_all,
            now,
            send_facts: send,
        };
        let prices = st.prices.clone();
        let price = move |_t: u8| prices.get(guid).unwrap_or(0);
        let env = ClickEnv {
            cursor_item: items::cursor_item(world).is_some(),
            // A gamble buy sends transaction 2: the handlers at
            // the gamble handlers write it.
            gamble_shop: st.gamble,
            repair_all_button_on: repair_all,
            repair_button_on: st.repair_mode,
            repair_all_price: st.prices.get(REPAIR_ALL_KEY).unwrap_or(0),
            price: &price,
        };
        if let ClickOutcome::Effects(e) = st.tx.click(&args, &env) {
            Self::apply(sh, st, world, strings, (send, now, item), e);
        }
    }

    /// A left press on one of the player's items while the shop is up: the
    /// grid / body-location click:
    /// repair through the confirm dialog. Returns whether the press was
    /// taken (repair button down, no cursor item, an item under it).
    pub(super) fn player_item_click(
        sh: &mut super::Shared,
        st: &mut ShopState,
        world: &ClientWorld,
        strings: &dyn StringLookup,
        item: Option<&ItemView>,
    ) -> bool {
        // D2RUST's own until the player-grid shop handler is specified
        // (the shop's callers are known, not when the grid hands a
        // click on): only the down repair button turns a click on one of
        // the player's items into the click; else the item is lifted.
        let Some(open) = st.open.filter(|_| st.repair_mode) else {
            return false;
        };
        let Some(it) = item.filter(|_| items::cursor_item(world).is_none()) else {
            return false;
        };
        let caller = if it.mode == items::mode::BODY {
            Caller::BodyLocation
        } else {
            Caller::GridClick(1)
        };
        Self::click(sh, st, open, world, strings, caller, Some(it), false);
        true
    }
}

/// The tip context of a store item hovered in the shop (the item tip
/// inputs): the inventory mode, a store item (owner flag 0), the
/// gamble store and the price text `Cost: ` + the price (gold fails).
///
/// D2RUST's own, unverified: the inventory mode is 4 while the repair button is
/// on (the repair mode), else 1 (the idle trade mode); the other buttons'
/// modes are not known. A store item without a price yet fails.
fn store_tip_ctx<'a>(
    base: crate::ui::item_tip_build::TipCtx<'a>,
    st: &ShopState,
    guid: u32,
    code: Option<[u8; 4]>,
) -> crate::ui::item_tip_build::TipCtx<'a> {
    use crate::ui::item_tip_build::PriceText;
    let gold = code.is_some_and(|c| &c == b"gld ");
    let price = match st.prices.get(guid) {
        Some(p) if !gold => PriceText::Price {
            label: crate::ui::item_tip_build::tid::COST,
            price: p as i32,
        },
        _ => PriceText::Fail,
    };
    crate::ui::item_tip_build::TipCtx {
        mode: if st.repair_mode { 4 } else { 1 },
        own_item: false,
        // The gamble flag of the item tip inputs: always 0.
        gamble: false,
        price,
        ..base
    }
}

/// The item's name: the first line of its tip (D2RUST's own until the
/// tip builder names it).
fn item_name(sh: &super::Shared, world: &ClientWorld, it: &ItemView) -> Option<Vec<u16>> {
    let tips = sh.items.tips.as_ref()?;
    let stream = items::stream(world, it.key)?;
    tips.lines(stream).into_iter().next().map(|l| l.text)
}

/// The hovered store item's tip, drawn with the other pop-ups after every
/// panel: it is not clipped or
/// covered by the shop panel's neighbours.
pub(super) fn draw_hover_tip(
    sh: &super::Shared,
    st: &ShopState,
    ctx: &UiCtx,
    out: &mut dyn UiDrawSink,
) {
    let Some(_) = st.open else {
        return;
    };
    let s = sh.config.screen;
    let (_, page) = tabs_of(ctx.world, st);
    let g = grid_record(sh);
    let list = page_items(sh, ctx.world, st, page);
    // The hovered store item's tip with the store context, else the bare price.
    let hovered = item_at(sh, &g, &list, sh.mouse);
    let tip = hovered.as_ref().and_then(|it| {
        let tips = sh.items.tips.as_ref()?;
        let stream = crate::bridge::items::stream(ctx.world, it.key)?;
        let me = crate::ui::item_tip_world::WorldUnit::local(ctx.world);
        let base = crate::ui::item_tip_world::hover_ctx(tips, ctx.world, me.as_ref(), it);
        let tc = store_tip_ctx(base, st, it.key.guid, it.code);
        let lines = tips.tip_lines(stream, &tc);
        (!lines.is_empty()).then_some(lines)
    });
    if let (Some(lines), Some(it)) = (tip, hovered.as_ref()) {
        // The hover anchor of the hover handler: the
        // item's cells, not the mouse (a store item's box top 93
        // over the row-0 item at y 123).
        let cell = (i32::from(g.cell_w), i32::from(g.cell_h));
        let size = sh.items.cells(&sh.tables.files, it, cell);
        let anchor = crate::ui::item_tip::TipAnchor::of_cell(
            (g.left, g.top),
            cell,
            (i32::from(it.x), i32::from(it.y)),
            size,
        );
        crate::ui::item_tip::draw_tip_at(&lines, anchor, (s.w, s.h), sh.fonts.as_ref(), out);
    } else if let (Some(it), Some(_)) = (hovered, &sh.fonts) {
        if let Some(price) = st.prices.get(it.key.guid) {
            out.push(text(
                utf16(&price.to_string()),
                sh.mouse.x + 14,
                sh.mouse.y + 14,
                crate::ui::panels::shop::FONT16,
                4,
            ));
        }
    }
}

/// Answers use the menu box's existing pointer bands.
pub(super) fn confirm_elements(bx: &MenuBox<ConfirmHandler>) -> Vec<crate::ui::ui_ids::UiElement> {
    let mut out = super::npc_box::menu_elements(bx, "confirm");
    for (element, item) in out.iter_mut().zip(bx.items.iter().filter(|i| i.selectable)) {
        element.id = match item.handler {
            Some(ConfirmHandler::Yes) => "shop.confirm.yes",
            _ => "shop.confirm.no",
        }
        .to_string();
    }
    out
}

impl Panel for ShopUi {
    fn ui_elements(&self, ctx: &UiCtx) -> Vec<crate::ui::ui_ids::UiElement> {
        let sh = self.sh.borrow();
        let st = self.st.borrow();
        let Some(open) = st.open else {
            return Vec::new();
        };
        let (panel, page) = self.panel(ctx.world, &st, open);
        let s = sh.config.screen;
        let g = grid_record(&sh);
        let mut out = g.ui_elements("shop");
        // The stocked items of the visible page, in stream order: a script
        // reaches "the first item for sale" without knowing which cell the
        // store put it in.
        for (n, it) in page_items(&sh, ctx.world, &st, page).iter().enumerate() {
            let (w, h) = footprint(&sh, it);
            let (x, y, cw, ch) = g.cell(i32::from(it.x), i32::from(it.y));
            let (w, h) = ((cw * w).min(g.right - x), (ch * h).min(g.bottom + 1 - y));
            if w > 0 && h > 0 {
                out.push(crate::ui::ui_ids::UiElement::new(
                    format!("shop.item({n})"),
                    crate::ui::geom::Rect::new(x, y, w as u16, h as u16),
                ));
            }
        }
        for (i, _) in panel.tabs.iter().enumerate().filter(|(_, t)| t.visible) {
            out.push(crate::ui::ui_ids::UiElement::new(
                format!("shop.tab({i})"),
                tab_rect(&s, i),
            ));
        }
        out
    }

    /// Drawn before the inventory.
    fn draw_before(&self) -> Option<PanelId> {
        Some(PanelId(u16::from(
            crate::ui::panels::inventory::UI_INVENTORY,
        )))
    }

    fn id(&self) -> PanelId {
        PanelId(u16::from(UI_SHOP))
    }

    /// The left half above the control panel.
    fn rect(&self) -> Rect {
        let s = self.sh.borrow().config.screen;
        s.left_side_rect()
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        let st = self.st.borrow();
        let Some(open) = st.open else {
            return;
        };
        let (panel, page) = self.panel(ctx.world, &st, open);
        let no = NoMeasure;
        let measure: &dyn TextMeasure = match &sh.fonts {
            Some(f) => f,
            None => &no,
        };
        // The art, the store grid (each item's tints, then its graphic, in list
        // order), the action buttons, then the tabs (`gen-ui-shop` tick 120).
        let env = sh.env();
        panel.draw_art(&sh.tables, &env, out);
        let s = sh.config.screen;
        let g = grid_record(&sh);
        let list = page_items(&sh, ctx.world, &st, page);
        sh.items
            .draw_grid_items(ctx.world, &sh.tables.files, &g, &list, sh.mouse, out);
        if let Some(f) = sh.tables.files.id("panel\\buysellbtn") {
            for (frame, x, y) in panel.button_cels(&s) {
                out.push(cel(f, frame, x, y));
            }
        }
        panel.draw_tabs(&sh.tables, &env, ctx.strings, measure, out);
        // After the shop draw: stat
        // 15 `goldbank`, layer 0, then the "Stash" label.
        let gold = match (&sh.fonts, crate::ui::original::local_player(ctx.world)) {
            (Some(f), Some((key, _))) => {
                let v = ctx.world.total(key, 15, 0);
                Some((v, f.width_a(1, &utf16(&v.to_string())).unwrap_or(0)))
            }
            _ => None,
        };
        let label = ctx
            .strings
            .get_id(crate::ui::panels::inv_gold::STR_STASH)
            .filter(|_| sh.fonts.is_some());
        crate::ui::panels::inv_gold::draw_shop_gold(&env, gold, label, out);
        // The repair-all total above the button bar (D2RUST's own).
        if let (true, Some(_)) = (REPAIR_CLASSES.contains(&open.npc_class), &sh.fonts) {
            if let Some(total) = st.prices.get(REPAIR_ALL_KEY).filter(|&t| t > 0) {
                out.push(text(
                    utf16(&total.to_string()),
                    s.sx() + 20,
                    s.h + s.sy() - 120,
                    crate::ui::panels::shop::FONT16,
                    4,
                ));
            }
        }
        // The hovered item's tip is the pop-up layer's ([`draw_hover_tip`]).
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        let (press, button, at) = match e {
            UiEvent::Press { button, at } => (true, button, at),
            UiEvent::Release { button, at } => (false, button, at),
            _ => return UiResponse::Ignored,
        };
        let mut sh = self.sh.borrow_mut();
        let mut st = self.st.borrow_mut();
        let Some(open) = st.open else {
            return UiResponse::Ignored;
        };
        let s = sh.config.screen;
        let (panel, page) = self.panel(ctx.world, &st, open);
        if !press {
            // Buttons stay down until clicked again; the repair-all button acts
            // on its release over it while pressed (WM_LBUTTONUP, base frame
            // 18).
            let i = button_at(&s, panel.buttons.len(), at);
            let rec = i.map(|i| shop_button_records(open.npc_class)[i]);
            if let (Some(i), Some(rec)) = (i, rec) {
                if rec.base_frame == 18 && st.pressed == Some(i) {
                    st.pressed = None;
                    Self::click(
                        &mut sh,
                        &mut st,
                        open,
                        ctx.world,
                        ctx.strings,
                        Caller::RepairAll(1),
                        None,
                        false,
                    );
                }
            }
            return UiResponse::Consumed;
        }
        // Tab row.
        if let Some(t) = (0..4).find(|&i| tab_rect(&s, i).contains(at)) {
            let (visible, active) = (panel.tabs[t].visible, panel.tabs[t].active);
            if visible && !active {
                st.page = t as u8;
                // Id 6 (the tab click).
                sh.outputs.push(PanelOutput::Sound(6));
            }
            return UiResponse::Consumed;
        }
        // Button bar (strict bounds, `X[3][i]` without the draw's − 1).
        if let Some(i) = button_at(&s, panel.buttons.len(), at) {
            {
                {
                    let rec = shop_button_records(open.npc_class)[i];
                    if rec.enabled {
                        // Not a constant-id click site:
                        // the panel click sound (id 0).
                        sh.outputs
                            .push(PanelOutput::Sound(crate::ui::original::CLICK_SOUND_ID));
                        // Another pressed button is released and
                        // this one pressed; this one pressed is released.
                        let was = st.pressed == Some(i);
                        st.pressed = (!was).then_some(i);
                        match rec.base_frame {
                            // Close (record 3 of the other traders).
                            10 => sh.outputs.push(PanelOutput::SetUi {
                                ui: id::NPC_SHOP,
                                mode: 1,
                                jump: false,
                            }),
                            // Repair one item: the button is the mode.
                            6 => st.repair_mode = !was,
                            // Repair all: pressed here, sent on the
                            // release (above).
                            18 => st.repair_mode = false,
                            _ => st.repair_mode = false,
                        }
                    }
                    return UiResponse::Consumed;
                }
            }
        }
        let g = grid_record(&sh);
        if !g.contains_mouse(at) {
            return UiResponse::Consumed;
        }
        let list = page_items(&sh, ctx.world, &st, page);
        let under = item_at(&sh, &g, &list, at);
        let cursor = items::cursor_item(ctx.world);
        let shift = sh.items.shift;
        let caller = match (button, cursor.as_ref(), under.as_ref()) {
            // A cursor item dropped on the store grid
            // sells it (quick).
            (PointerButton::Left, Some(c), _) => Some((Caller::DropOnStoreGrid, c.clone())),
            // The right click buys at once (quick).
            (PointerButton::Right, None, Some(it)) => {
                Some((Caller::StoreGridRightClick, it.clone()))
            }
            // The left click goes through the confirm
            // dialog.
            (PointerButton::Left, None, Some(it)) => Some((Caller::GridClick(0), it.clone())),
            _ => None,
        };
        if let Some((c, it)) = caller {
            Self::click(
                &mut sh,
                &mut st,
                open,
                ctx.world,
                ctx.strings,
                c,
                Some(&it),
                shift,
            );
        }
        UiResponse::Consumed
    }
}

impl OriginalUi {
    /// Opens the shop for the NPC with this GUID and class (the NPC menu's
    /// Trade option): the shop and inventory panels come
    /// up; the store items arrive as S→C 0x9C action 0x0B.
    pub fn open_shop(&mut self, npc_guid: u32, npc_class: u32) {
        let gamble = self.npcm.borrow().gamble;
        {
            let mut st = self.shop.borrow_mut();
            st.gamble = gamble;
            st.open = Some(ShopOpen {
                npc_guid,
                npc_class,
            });
            st.tx = ShopTx::default();
            st.confirm = None;
            st.note = false;
            st.pressed = None;
            st.repair_mode = false;
            st.closed = false;
            // The start page of the class.
            st.page = shop_start_page(npc_class).0;
        }
        // The shop is the full-slot ui 0x0C;
        // it draws the inventory family itself (ui 1 stays off, ui 8 on).
        // D2RUST's own: a refused open (the gate) leaves the shop state up
        // and `shop_poll` closes it again.
        self.shared.borrow_mut().items.store_npc = Some(npc_guid);
        let _ = self.set_ui(u32::from(id::NPC_SHOP), 0, false);
    }

    /// The prices the server host publishes ([`ShopPrices`]).
    pub fn set_shop_prices(&mut self, prices: ShopPrices) {
        self.shop.borrow_mut().prices = prices;
    }

    /// The shop state (tests).
    pub fn shop_state(&self) -> std::cell::Ref<'_, ShopState> {
        self.shop.borrow()
    }

    /// The centre of the panel cell the shop shows store item `guid` at
    /// on the current page (tests).
    pub fn store_item_point(&self, world: &ClientWorld, guid: u32) -> Option<Point> {
        let sh = self.shared.borrow();
        let st = self.shop.borrow();
        st.open?;
        let g = grid_record(&sh);
        let (_, page) = tabs_of(world, &st);
        let it = page_items(&sh, world, &st, page)
            .into_iter()
            .find(|i| i.key.guid == guid)?;
        let (l, t, w, h) = g.cell(i32::from(it.x), i32::from(it.y));
        Some(Point::new(l + w / 2, t + h / 2))
    }

    /// The tip lines of store item `guid` as the shop draws them on
    /// hover, with the store context ([`store_tip_ctx`]; tests).
    pub fn store_tip_lines(&self, world: &ClientWorld, guid: u32) -> Vec<String> {
        let sh = self.shared.borrow();
        let st = self.shop.borrow();
        let Some(tips) = sh.items.tips.as_ref() else {
            return Vec::new();
        };
        let Some(it) = items::store_items(world)
            .into_iter()
            .find(|i| i.key.guid == guid)
        else {
            return Vec::new();
        };
        let Some(stream) = items::stream(world, it.key) else {
            return Vec::new();
        };
        let me = crate::ui::item_tip_world::WorldUnit::local(world);
        let base = crate::ui::item_tip_world::hover_ctx(tips, world, me.as_ref(), &it);
        let tc = store_tip_ctx(base, &st, guid, it.code);
        tips.tip_lines(stream, &tc)
            .iter()
            .map(|l| String::from_utf16_lossy(&l.text))
            .collect()
    }

    /// The shown page's store items as (GUID, x, y, w, h): the stream's
    /// cell and the footprint (tests).
    pub fn store_item_cells(&self, world: &ClientWorld) -> Vec<(u32, i32, i32, i32, i32)> {
        let sh = self.shared.borrow();
        let st = self.shop.borrow();
        if st.open.is_none() {
            return Vec::new();
        }
        let (_, page) = tabs_of(world, &st);
        page_items(&sh, world, &st, page)
            .iter()
            .map(|i| {
                let (w, h) = footprint(&sh, i);
                (i.key.guid, i32::from(i.x), i32::from(i.y), w, h)
            })
            .collect()
    }

    /// The store item the shop's hit test finds at grid cell (x, y)
    /// (`item_at` at the cell's centre; tests).
    pub fn store_item_at_cell(&self, world: &ClientWorld, x: i32, y: i32) -> Option<u32> {
        let sh = self.shared.borrow();
        let st = self.shop.borrow();
        st.open?;
        let g = grid_record(&sh);
        let (_, page) = tabs_of(world, &st);
        let list = page_items(&sh, world, &st, page);
        let (l, t, w, h) = g.cell(x, y);
        item_at(&sh, &g, &list, Point::new(l + w / 2, t + h / 2)).map(|i| i.key.guid)
    }

    /// The centre of store grid cell (x, y) on screen (tests).
    pub fn store_cell_point(&self, x: i32, y: i32) -> Point {
        let g = grid_record(&self.shared.borrow());
        let (l, t, w, h) = g.cell(x, y);
        Point::new(l + w / 2, t + h / 2)
    }

    /// The centre of the confirm dialog's Yes (`yes`) or No item (tests).
    pub fn shop_confirm_point(&self, yes: bool) -> Option<Point> {
        let st = self.shop.borrow();
        let c = st.confirm.as_ref()?;
        let want = if yes {
            ConfirmHandler::Yes
        } else {
            ConfirmHandler::No
        };
        let i = c.bx.items.iter().position(|i| i.handler == Some(want))?;
        let r = super::npc_box::item_rect(&c.bx, i);
        Some(Point::new((r.l + r.r) / 2, (r.t + r.b) / 2))
    }

    /// Per frame: opens the shop when a trade's store items arrived (for
    /// the trader nearest to the local player, D2RUST's own) and ends the
    /// interaction (C→S 0x30) when the shop's UI state went off.
    pub fn shop_poll(&mut self, world: &ClientWorld, root: &mut UiRoot) {
        // D2RUST's own, unverified: the Trade / Gamble choice opens the shop at
        // once, so its close ends the interaction (C→S 0x30) even when no store
        // item arrives.
        let request = self.npcm.borrow_mut().shop_request.take();
        if let Some((guid, class)) = request {
            if self.shop.borrow().open.is_none() {
                self.open_shop(guid, class);
                // The store records newer than the choice are this trade's
                // (a closed shop's late copy is not; q-smoke-town).
                if let Some((_, _, floor)) = self.npcm.borrow_mut().shop_for.take() {
                    self.shop.borrow_mut().floor = floor;
                }
            }
        }
        let (open, serial) = {
            let st = self.shop.borrow();
            (st.open, st.serial)
        };
        if world.store_serial != serial {
            self.shop.borrow_mut().serial = world.store_serial;
            if open.is_none() {
                // The trader the menu chose, else the nearest one (the
                // model's local position is the last placement, not the
                // predicted walk: D2RUST's own, unverified).
                let chosen = self.npcm.borrow_mut().shop_for.take();
                if let Some((guid, class, floor)) = chosen {
                    self.open_shop(guid, class);
                    // The records of an earlier trade (a closed shop's late
                    // copy) are not this store's.
                    self.shop.borrow_mut().floor = floor;
                } else if !self.shop.borrow().closed {
                    // A closed shop's late records (the copy of an item sold
                    // just before the close) are not a new trade.
                    if let Some((guid, class)) = nearest_trader(world) {
                        self.open_shop(guid, class);
                    }
                }
            }
        }
        let open = self.shop.borrow().open;
        if let Some(o) = open.filter(|_| !self.is_open(id::NPC_SHOP)) {
            {
                let mut st = self.shop.borrow_mut();
                st.open = None;
                self.shared.borrow_mut().items.store_npc = None;
                st.repair_mode = false;
                st.floor = world.store_serial;
                st.closed = true;
            }
            // The interaction's end sends
            // 30 [1 u32][GUID u32], then `SetUIState(8, off)`.
            root.queue_intent(ClientIntent(msg_chat_end(o.npc_guid).to_vec()));
            self.shop.borrow_mut().confirm = None;
            let _ = self.set_ui(u32::from(id::NPC_MENU), 1, false);
        }
        self.sync_root(root);
    }
}

/// The trader nearest to the local player: a monster unit of a trading
/// class.
fn nearest_trader(world: &ClientWorld) -> Option<(u32, u32)> {
    let (px, py) = world.local().and_then(|u| u.position).unwrap_or((0, 0));
    let d = |u: &crate::bridge::world::ClientUnit| {
        let (x, y) = u.position.unwrap_or((u16::MAX, u16::MAX));
        (i64::from(x) - i64::from(px)).pow(2) + (i64::from(y) - i64::from(py)).pow(2)
    };
    world
        .units
        .values()
        .filter(|u| u.key.unit_type == MONSTER && trades(u.class))
        .min_by_key(|u| d(u))
        .map(|u| (u.key.guid, u.class))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::UnitKey;

    fn at(guid: u32, x: u16, y: u16) -> ItemView {
        ItemView {
            key: UnitKey::new(4, guid),
            code: Some(*b"hp1 "),
            flags: 0,
            mode: 0,
            body: 0,
            page: 1,
            x,
            y,
            owner: None,
            store: true,
            store_seq: guid,
            gold: None,
        }
    }

    #[test]
    fn store_items_stay_at_the_servers_cells() {
        // The sim placed these with gaps (first free spot, 2 x 3 items);
        // the shop must not pack them to the top-left.
        let list = vec![at(1, 4, 0), at(2, 0, 5), at(3, 9, 9), at(4, 9, 8)];
        let size = |i: &ItemView| if i.key.guid == 4 { (2, 3) } else { (1, 1) };
        let kept = in_grid(list, size);
        let cells: Vec<_> = kept.iter().map(|i| (i.key.guid, i.x, i.y)).collect();
        // Guid 4 (2 x 3 at (9, 8)) leaves the grid and is not drawn.
        assert_eq!(cells, vec![(1, 4, 0), (2, 0, 5), (3, 9, 9)]);
    }

    #[test]
    fn the_store_grid_is_the_monster_record_placed_as_measured() {
        use crate::ui::layout::Screen;
        assert_eq!(store_record(&Screen::R640), 5);
        assert_eq!(store_record(&Screen::R800), 21);
        let g = fallback_store_grid(&Screen::R640);
        assert_eq!((g.left, g.right, g.top, g.bottom), (16, 305, 63, 352));
        let g = fallback_store_grid(&Screen::R800);
        // Column 9 of row 0 at x 357, y 123 (gen-ui-shop tick 120).
        assert_eq!(g.cell(9, 0), (357, 123, 29, 29));
    }
}
