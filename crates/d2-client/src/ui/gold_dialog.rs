//! The inventory gold button and the drop-gold dialog in play: a press in
//! the button rectangle sets the pressed flag, a release in it opens the
//! dialog of kind 1 (drop; `GoldButtons`, `GoldDialog`, `ok_action` of
//! [`crate::ui::gold`]); OK sends C→S 0x50 `DropGold` and
//! Cancel closes. While the dialog is open it is modal: it takes every
//! pointer event, every typed character (digits only, the edit box rules)
//! and every action, so the belt and menu keys do nothing.
//!
//! Preview fills, each `// D2RUST's own, unverified` (M22):
//! - the dialog box art, the spinner and the OK / Cancel art are
//!   not drawn: the box is tiles of the synthetic fill file (as the Esc
//!   menu's), the prompt, the value and the two buttons are Font16 text
//!   in English at positions chosen here;
//! - Enter is OK and Escape is Cancel (the box family's keys;
//!   family; the box rules are not bound here);
//! - the gold button's sound 4 is not played (sound is deferred).
//!
//! The client decides nothing: OK is a request the server checks.

use d2_proto::client::{ClickButton, DropGold};

use super::hud::{FILL_FILE, FILL_H, FILL_W};
use super::{left, SharedRef};
use crate::bridge::items;
use crate::ui::draw::{ImageRef, ImageRequest, TextRequest, TextStyle, UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect};
use crate::ui::gold::{
    can_open, inventory_gold_hit, ok_action, CharResult, GoldButtons, GoldDialog, GoldKind,
    GoldLayout, GoldSend,
};
use crate::ui::panel::{ClientIntent, Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use crate::ui::panels::inventory::UI_INVENTORY;
use crate::ui::panels::stash_cube::UI_STASH;
use crate::ui::panels::{utf16, PanelOutput};
use crate::ui::text::TextOpts;

/// The adapter's id: not a UI state, open for good.
pub const GOLD_PANEL: PanelId = PanelId(0x110);

/// The fill file's dark frame (made by `hud::fill_frames`).
const DARK: u32 = 4;
const BOX_W: i32 = 300;
const BOX_H: i32 = 170;
const FONT: u16 = 1;
const COLOR_REST: u16 = 0;
const COLOR_HOVER: u16 = 3;

/// The gold button and dialog state.
#[derive(Clone, Debug, Default)]
pub struct GoldState {
    pub buttons: GoldButtons,
    pub dialog: Option<GoldDialog>,
}

fn box_left(w: i32) -> i32 {
    (w - BOX_W) / 2
}

fn box_top(h: i32) -> i32 {
    (h - BOX_H) / 2
}

/// The OK and Cancel rectangles for a frame of `w` × `h`.
fn buttons(w: i32, h: i32) -> (Rect, Rect) {
    let (x, y) = (box_left(w), box_top(h) + 115);
    (
        Rect::new(x + 30, y, 110, 36),
        Rect::new(x + BOX_W - 140, y, 110, 36),
    )
}

/// What a typed character or a click asks of an open dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    Ok,
    Cancel,
}

/// The adapter (installed above the inventory, below the Esc menu).
pub struct GoldDialogUi {
    pub(super) sh: SharedRef,
}

impl GoldDialogUi {
    fn layout(&self) -> GoldLayout {
        let sh = self.sh.borrow();
        let s = sh.config.screen;
        GoldLayout {
            sx: s.sx(),
            sy: s.sy(),
            w: s.w,
            h: s.h,
            expansion: sh.env().exp,
        }
    }

    /// Closes the dialog; with `Ask::Ok` the drop request goes out.
    fn close(&mut self, ask: Ask, ctx: &UiCtx) {
        let mut sh = self.sh.borrow_mut();
        let Some(mut d) = sh.gold.dialog.take() else {
            return;
        };
        let v = d.close();
        sh.gold.buttons.dialog_flag = false;
        if ask != Ask::Ok {
            return;
        }
        let guid = ctx.world.local().map(|u| u.key.guid);
        match ok_action(d.kind, v, guid) {
            GoldSend::DropGold { guid, amount } => {
                sh.outputs
                    .push(PanelOutput::Intent(ClientIntent::from_message(&DropGold {
                        unit: guid,
                        amount,
                    })))
            }
            // Kinds 3, 4: C→S 0x4F 0x14 / 0x13 [amount >> 16, amount & 0xFFFF].
            GoldSend::StashButton { button, p1, p2, .. } => {
                sh.outputs
                    .push(PanelOutput::Intent(ClientIntent::from_message(
                        &ClickButton { button, p1, p2 },
                    )));
                sh.outputs.push(PanelOutput::Sound(0xDD));
            }
            _ => {}
        }
    }

    fn dialog_event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        let (w, h) = {
            let s = self.sh.borrow().config.screen;
            (s.w, s.h)
        };
        let (ok, cancel) = buttons(w, h);
        let ask = match e {
            UiEvent::Release { at, .. } if ok.contains(at) => Some(Ask::Ok),
            UiEvent::Release { at, .. } if cancel.contains(at) => Some(Ask::Cancel),
            UiEvent::Char(0x0D) => Some(Ask::Ok),
            UiEvent::Char(0x1B) => Some(Ask::Cancel),
            UiEvent::Action(a)
                if a == crate::ui::ActionId(crate::controls::Action::GameMenu.index() as u16) =>
            {
                Some(Ask::Cancel)
            }
            _ => None,
        };
        if let Some(ask) = ask {
            self.close(ask, ctx);
            return UiResponse::Consumed;
        }
        if let UiEvent::Char(c) = e {
            let mut sh = self.sh.borrow_mut();
            if let Some(d) = sh.gold.dialog.as_mut() {
                if d.edit.char(u32::from(c), false, &|_| false) == CharResult::Consumed {
                    return UiResponse::Consumed;
                }
            }
        }
        match e {
            UiEvent::Press { .. }
            | UiEvent::Release { .. }
            | UiEvent::Wheel { .. }
            | UiEvent::Char(_)
            | UiEvent::Action(_) => UiResponse::Consumed,
            _ => UiResponse::Ignored,
        }
    }

    /// The inventory gold button, dialog closed.
    fn button_event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        // The stash release handler also owns the button.
        let open = {
            let sh = self.sh.borrow();
            sh.states.is_open(UI_INVENTORY) || sh.states.is_open(UI_STASH)
        };
        if !open {
            return UiResponse::Ignored;
        }
        let Some((down, at)) = left(e) else {
            return UiResponse::Ignored;
        };
        let l = self.layout();
        let cursor = items::cursor_item(ctx.world).is_some();
        let in_rect = inventory_gold_hit(&l, at, cursor);
        let mut sh = self.sh.borrow_mut();
        if down {
            return if sh.gold.buttons.press_inventory(in_rect).consumed {
                UiResponse::Consumed
            } else {
                UiResponse::Ignored
            };
        }
        let was_pressed = sh.gold.buttons.inv_pressed;
        let kind = sh.gold.buttons.release_inventory(in_rect);
        if let Some(kind) = kind {
            // With the stash open the same button deposits (kind 3).
            let kind = if sh.states.is_open(UI_STASH) && kind == GoldKind::Drop {
                GoldKind::Deposit
            } else {
                kind
            };
            open_dialog(&mut sh, ctx.world, kind, cursor);
        }
        if was_pressed {
            UiResponse::Consumed
        } else {
            UiResponse::Ignored
        }
    }
}

impl Panel for GoldDialogUi {
    fn id(&self) -> PanelId {
        GOLD_PANEL
    }

    fn rect(&self) -> Rect {
        self.sh.borrow().config.screen.frame_rect()
    }

    fn draw(&self, _ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        let Some(d) = sh.gold.dialog.as_ref() else {
            return;
        };
        let (w, h) = (sh.config.screen.w, sh.config.screen.h);
        let clip = sh.config.screen.rect();
        let (bx, by) = (box_left(w), box_top(h));
        if let Some(file) = sh.tables.files.id(FILL_FILE) {
            let (tw, th) = (FILL_W as i32, FILL_H as i32);
            let mut y = by;
            while y < by + BOX_H {
                let mut x = bx;
                while x < bx + BOX_W {
                    let cw = (bx + BOX_W - x).min(tw);
                    let ch = (by + BOX_H - y).min(th);
                    out.push(UiDraw::Image(ImageRequest {
                        image: ImageRef { file, frame: DARK },
                        at: Point::new(x, y),
                        clip: Rect::new(x, y, cw as u16, ch as u16),
                        look: crate::ui::CelLook::PLAIN,
                        call: crate::ui::draw::CelCall::Draw,
                    }));
                    x += tw;
                }
                y += th;
            }
        }
        let line = |s: &str, y: i32, color: u16, out: &mut dyn UiDrawSink| {
            out.push(UiDraw::Text(TextRequest {
                text: utf16(s),
                at: Point::new(bx, y),
                style: TextStyle { font: FONT, color },
                opts: TextOpts::Draw {
                    centered: true,
                    block_w: Some(BOX_W),
                    mode: 5,
                },
                clip,
            }));
        };
        // String 4033 `strDropGoldHowMuch`, in English until the string
        // tables are bound (D2RUST's own, unverified).
        line("How much gold", by + 30, COLOR_REST, out);
        line("do you want to drop?", by + 55, COLOR_REST, out);
        let shown = String::from_utf8_lossy(&d.edit.text).into_owned();
        line(&format!("{shown}_"), by + 90, COLOR_REST, out);
        let (ok, cancel) = buttons(w, h);
        for (r, label) in [(ok, "OK"), (cancel, "Cancel")] {
            let color = if r.contains(sh.mouse) {
                COLOR_HOVER
            } else {
                COLOR_REST
            };
            out.push(UiDraw::Text(TextRequest {
                text: utf16(label),
                at: Point::new(r.x, r.y + 30),
                style: TextStyle { font: FONT, color },
                opts: TextOpts::Draw {
                    centered: true,
                    block_w: Some(i32::from(r.w)),
                    mode: 5,
                },
                clip,
            }));
        }
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        if self.sh.borrow().gold.dialog.is_some() {
            self.dialog_event(e, ctx)
        } else {
            self.button_event(e, ctx)
        }
    }
}

/// Opens the dialog of `kind` when `can_open` allows: the
/// maximum is the player's gold, the stash gold for kind 4.
pub(super) fn open_dialog(
    sh: &mut super::Shared,
    world: &crate::bridge::world::ClientWorld,
    kind: GoldKind,
    cursor: bool,
) {
    let Some((key, _)) = super::local_player(world) else {
        return;
    };
    if can_open(true, cursor, sh.gold.dialog.is_some()) {
        // The full stat is the maximum.
        let max = world.total(key, kind.max_stat(), 0).max(0) as u32;
        let (d, fx) = GoldDialog::open(kind, max, false);
        sh.gold.buttons.dialog_flag = fx.set_dialog_flag;
        sh.gold.dialog = Some(d);
    }
}

/// The kinds the inventory button opens in play (the stash open makes it
/// a deposit, `open_dialog`).
pub const INVENTORY_KIND: GoldKind = GoldKind::Drop;
