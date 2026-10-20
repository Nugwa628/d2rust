//! UI core: D2RUST's integer-pixel panel framework (not `bevy_ui`).
//! Panels live in a [`UiRoot`] in a fixed order, answer integer hit tests,
//! take [`UiEvent`]s in 800×600 frame coordinates and emit plain draw
//! requests ([`UiDraw`]) to a [`UiDrawSink`]. A panel never decides an
//! outcome: a click becomes a [`ClientIntent`] (an encoded C→S message)
//! that only the root forwards to the bridge. Clickable elements can be
//! named for test scripts ([`ui_ids`]).
//!
//! Everything here is plain Rust without Bevy types except [`edge`], the
//! input edge that reads the cursor from a Bevy `Window`.
//!
//! The game's own panels (art and layout, open/close rules, text layout,
//! grid cells, cursor) live in their modules below ([`original`] and the
//! panel files); this module holds only the framework they share.
//!
//! A mod game's HUD lines are [`mod_hud`].

pub mod automap;
pub mod box_border;
pub mod char_desc;
pub mod char_feed;
pub mod cursor;
pub mod draw;
pub mod edge;
pub mod edit_box;
pub mod frame;
pub mod front_end;
pub mod geom;
pub mod gold;
pub mod hire_list;
pub mod inv_grid;
pub mod item_tip;
pub mod item_tip_build;
pub mod item_tip_desc;
pub mod item_tip_props;
pub mod item_tip_set;
pub mod item_tip_world;
pub mod layout;
pub mod messages;
pub mod mod_hud;
pub mod original;
pub mod panel;
pub mod panels;
pub mod quest_log;
pub mod root;
pub mod show_items;
pub mod skill_desc;
pub mod skill_desc_more;
pub mod skill_tree_ui;
pub mod states;
pub mod text;
pub mod ui_clock;
pub mod ui_ids;
pub mod wformat;
pub mod widget;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_c2skills;
#[cfg(test)]
mod tests_c2ui;
#[cfg(test)]
mod tests_fdesc;

pub use draw::{
    CelLook, ImageRef, ImageRequest, LineRequest, RectRequest, Remap, TextRequest, TextStyle,
    UiDraw, UiDrawSink, DRAW_MODE_OPAQUE,
};
pub use frame::{scale_mode, set_scale_mode, FrameError, FramePos, Presentation, ScaleMode};
pub use geom::{Point, Rect, FRAME, FRAME_H, FRAME_W};
pub use panel::{
    ActionId, ClientIntent, NoStrings, Panel, PanelId, PointerButton, StringLookup, UiCtx, UiEvent,
    UiInput, UiResponse, WidgetId,
};
pub use root::{NoPanelRules, PanelRules, Routed, UiError, UiHit, UiRoot};
pub use text::{
    font_info, layout_text, FontInfo, GlyphDraw, GlyphLookup, GlyphPlacement, OriginalText,
    TextError, TextOpts, TextRules, FONTS,
};
