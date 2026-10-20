//! The HUD lines a mod game adds (`crate::mods::HudLines`, seams MS14a),
//! drawn as one panel of the play UI.
//!
//! The panel is added to the root only when the game's [`ClientMods`]
//! has a HUD line source, so D2RUST's own UI has the same panels as
//! without mods. It sits right above the control panel's overlays (the
//! HUD), below the game's boxes, menus and the cursor, is open for good,
//! takes no input and is never hit.

use super::draw::{TextRequest, TextStyle, UiDraw, UiDrawSink};
use super::geom::{Point, Rect, FRAME};
use super::panel::{Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use super::text::TextOpts;
use crate::mods::ClientMods;

/// The panel's id: not a UI state.
pub const MOD_HUD_PANEL: PanelId = PanelId(0x116);

/// Draws the mod game's HUD lines.
pub struct ModHudUi {
    pub mods: ClientMods,
}

impl Panel for ModHudUi {
    fn id(&self) -> PanelId {
        MOD_HUD_PANEL
    }

    /// Empty: the panel is never under the pointer, so hover and clicks
    /// go to the panels below as without it.
    fn rect(&self) -> Rect {
        Rect::default()
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        for l in self.mods.hud_lines(ctx.world) {
            out.push(UiDraw::Text(TextRequest {
                text: l.text,
                at: l.at,
                style: TextStyle {
                    font: l.font,
                    color: l.color,
                },
                opts: TextOpts::default(),
                clip: FRAME,
            }));
        }
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, _e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        UiResponse::Ignored
    }
}
