//! The Horadric Cube (ui 0x1A) installed in the play UI: the
//! [`CubePanel`] art, close and transmute buttons, the page-3 items and
//! grid click ([`ItemsUi::draw_cube`], [`ItemsUi::press_cube`]) and the
//! press / release rules ([`StashCubeInput`]).
//! S→C 0x77 0x15 opens the state (`msg_ui`); the C→S intents (0x4F 0x17,
//! 0x4F 0x18, the item moves) leave through the root. The client decides
//! nothing: the server transmutes.
//!
//! The transmute animation ([`HoradricAnim`]) steps on the UI clock
//! ([`super::ui_clock`]) and the open clears the close latch (`StashCubeInput::cube_opened`).
//!
//! The animation starts when an `hst ` / `qf2 ` lands in the local
//! player's page 3 (S→C 0x9C: `Output::HoradricItem`
//! → the quest-record check), at the cube's next draw; the
//! transmute button starts nothing.
//!
//! Preview fills (D2RUST's own, unverified): the 70 ms step reads the UI clock (the
//! wall clock in live play, so the animation lasts the same at any frame rate; 40 ms
//! per bridge frame in check runs). The cel draws in mode 3
//! (additive, [`HORADRIC_LOOK`]). The cube-gone close runs once per pass
//! ([`OriginalUi::cube_poll`]).

use super::{OriginalUi, OriginalUiError, SharedRef};
use crate::bridge::items;
use crate::bridge::world::ClientWorld;
use crate::ui::draw::{ImageRef, ImageRequest, UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect};
use crate::ui::panel::{Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use crate::ui::panels;
use crate::ui::panels::cube_items::cube_player_ok;
use crate::ui::panels::stash_cube::{horadric_pos, CubePanel, STR_TRANSMUTE, UI_CUBE};
use crate::ui::panels::stash_input::{
    cube_close_hit, cube_tooltips, transmute_hit, Pointer, StashCubeInput,
};
use crate::ui::panels::PanelOutput;
use crate::ui::root::UiRoot;
use crate::ui::states::id;
use crate::ui::PointerButton;

/// `strClose` and the tool tips' font.
pub(super) const STR_CLOSE: u16 = 4144;
const TIP_FONT: u16 = 1;

/// `menu\horadric` (31 frames), registered with the panel files.
const HORADRIC_FILE: &str = "menu\\horadric";
/// The animation cel draws in mode 3 (additive), light 0xFF, no
/// remap.
pub const HORADRIC_LOOK: crate::ui::CelLook = crate::ui::CelLook {
    mode: 3,
    remap: crate::ui::Remap::None,
};

pub(super) fn cube_files() -> [String; 1] {
    [HORADRIC_FILE.to_string()]
}

/// The cube adapter (ui 0x1A, left half above the control panel).
pub(super) struct CubeUi {
    pub(super) sh: SharedRef,
    pub(super) input: StashCubeInput,
}

impl Panel for CubeUi {
    /// Drawn before the inventory.
    fn draw_before(&self) -> Option<PanelId> {
        Some(PanelId(u16::from(
            crate::ui::panels::inventory::UI_INVENTORY,
        )))
    }

    fn id(&self) -> PanelId {
        PanelId(u16::from(UI_CUBE))
    }

    /// The left half above the control panel, plus the inventory close
    /// rectangle the stash / cube mouse handlers test first: the frame
    /// above the control panel, and [`Self::event`] lets
    /// every other point go on.
    fn rect(&self) -> Rect {
        let s = self.sh.borrow().config.screen;
        s.frame_rect_h(s.h - 48)
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        let env = sh.env();
        let panel = CubePanel {
            close_pressed: self.input.cube_close_pressed,
            transmute_pressed: self.input.transmute_pressed,
        };
        // The dead / no-player close (two 0x4F 0x17) is not sent from a draw:
        // outputs leave only after an event, so [`OriginalUi::cube_poll`] sends
        // it. The panel draws nothing that frame.
        if !cube_player_ok(ctx.world) {
            return;
        }
        panel.draw_art(&sh.tables, &env, out);
        // Step on the (wrapping, 32-bit) millisecond tick, draw the
        // cel, and hold the grid back for the first 14 steps.
        let mut anim = sh.cube_anim.get();
        let now = sh.ui_ms32(ctx.tick);
        // Flag, n:= 0, stamp:= now.
        if sh.horadric_start.replace(false) {
            anim.start(now);
        }
        anim.step(now);
        sh.cube_anim.set(anim);
        if let (Some(n), Some(file)) = (anim.frame(), sh.tables.files.id(HORADRIC_FILE)) {
            let (x, y) = horadric_pos(&sh.config.screen);
            let s = sh.config.screen;
            out.push(UiDraw::Image(ImageRequest {
                image: ImageRef { file, frame: n },
                at: Point::new(x, y),
                clip: Rect::new(0, 0, s.w as u16, s.h as u16),
                look: HORADRIC_LOOK,
                call: crate::ui::draw::CelCall::Draw,
            }));
        }
        if anim.grid_visible() {
            let g = sh.items.cube_grid(&sh.config.screen);
            // The cube's placement tint.
            sh.items.draw_placement_tint(
                ctx.world,
                &sh.tables.files,
                &g,
                crate::ui::panels::cube_items::CUBE_PAGE,
                sh.mouse,
                sh.config.screen.h,
                out,
            );
            sh.items
                .draw_cube(ctx.world, &sh.tables.files, &g, sh.mouse, out);
        }
        // The buttons after the grid.
        panel.draw_buttons(&sh.tables, &env, out);
        // The button tool tips: the strict
        // button rectangles, `strClose` / `strUiMenu2` "Transmute". Drawn
        // when the string table is loaded (font 1, D2RUST's own, unverified).
        let s = sh.config.screen;
        let tip = if cube_close_hit(&s, sh.mouse) {
            Some((STR_CLOSE, true))
        } else if transmute_hit(&s, sh.mouse) {
            Some((STR_TRANSMUTE, false))
        } else {
            None
        };
        if let Some(text) = tip.and_then(|(id, close)| Some((ctx.strings.get_id(id)?, close))) {
            let (t, close) = text;
            let w = sh
                .fonts
                .as_ref()
                .and_then(|f| f.width_a(TIP_FONT, t))
                .unwrap_or(0);
            let (c, m) = cube_tooltips(&s, w, w);
            let at = if close { c } else { m };
            out.push(panels::text(t.to_vec(), at.x, at.y, TIP_FONT, 0));
        }
    }

    fn ui_elements(&self, ctx: &UiCtx) -> Vec<crate::ui::ui_ids::UiElement> {
        if !cube_player_ok(ctx.world) {
            return Vec::new();
        }
        let sh = self.sh.borrow();
        let s = sh.config.screen;
        let mut out = if sh.cube_anim.get().grid_visible() {
            sh.items.cube_grid(&s).ui_elements("cube")
        } else {
            Vec::new()
        };
        out.push(crate::ui::ui_ids::UiElement::new(
            "cube.close",
            crate::ui::panels::stash_input::cube_close_rect(&s),
        ));
        out.push(crate::ui::ui_ids::UiElement::new(
            "cube.transmute",
            crate::ui::panels::stash_input::transmute_rect(&s),
        ));
        out
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        let (down, at) = match e {
            UiEvent::Press {
                button: PointerButton::Left,
                at,
            } => (true, at),
            UiEvent::Release {
                button: PointerButton::Left,
                at,
            } => (false, at),
            _ => return UiResponse::Ignored,
        };
        let mut sh = self.sh.borrow_mut();
        if std::mem::take(&mut sh.cube_opened) {
            self.input.cube_opened();
        }
        let s = sh.config.screen;
        // The inventory close rectangle (the belt popup
        // covering it is not tested: D2RUST's own, the popup is the HUD's).
        let in_inv_close = crate::ui::panels::inventory::close_rect(&sh.tables, &s)
            .is_some_and(|r| r.contains(at));
        let left = s.left_side_rect();
        if !in_inv_close && !left.contains(at) {
            return UiResponse::Ignored;
        }
        let ptr = Pointer {
            at,
            in_inv_close,
            cursor_item: items::cursor_item(ctx.world).is_some(),
        };
        let eff = if down {
            self.input.cube_down(&s, &ptr)
        } else {
            self.input.cube_up(&s, &ptr)
        };
        if eff.sound4 {
            sh.outputs.push(PanelOutput::Sound(4));
        }
        // The transmute button starts nothing; the
        // close clears the animation.
        if eff
            .outputs
            .iter()
            .any(|o| matches!(o, PanelOutput::SetUi { .. }))
        {
            sh.cube_anim.set(Default::default());
        }
        sh.outputs.extend(eff.outputs);
        if eff.consumed {
            return UiResponse::Consumed;
        }
        if down {
            let g = sh.items.cube_grid(&s);
            let out = sh.items.press_cube(ctx.world, &sh.tables.files, &g, at);
            sh.outputs.extend(out);
        }
        UiResponse::Consumed
    }
}

impl OriginalUi {
    /// Once per pass: when ui 0x1A is open and the local player is missing
    /// or dead (mode 0x11), the close
    /// (a missing cube does not close it):
    /// `SetUIState(0x1A, off)`, then C→S 0x4F 0x17 twice (the latched
    /// close, then the unconditional one). The state was just open, so
    /// the close latch is the open's (clear). Nothing otherwise.
    pub fn cube_poll(
        &mut self,
        world: &ClientWorld,
        root: &mut UiRoot,
    ) -> Result<(), OriginalUiError> {
        if !self.is_open(id::CUBE) || cube_player_ok(world) {
            return Ok(());
        }
        for o in StashCubeInput::default().cube_gone(true) {
            match o {
                PanelOutput::Intent(i) => root.queue_intent(i),
                PanelOutput::SetUi { ui, mode, jump } => {
                    // The two 0x4F 0x17 above include the hook's.
                    self.set_ui_from(u32::from(ui), u32::from(mode), jump, Some(ui))?;
                }
                PanelOutput::Sound(_)
                | PanelOutput::PlayerEvent(_)
                | PanelOutput::DialogSpeechEnd => {}
            }
        }
        self.flush_hooks(root);
        self.sync_root(root);
        Ok(())
    }
}

#[cfg(test)]
#[path = "cube_ui_tests.rs"]
mod tests;
