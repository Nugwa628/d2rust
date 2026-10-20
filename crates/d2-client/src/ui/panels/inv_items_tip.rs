//! The inventory panel's item tool tip and the identify cursor.
//!
//! - Hover: the item under the mouse in the
//!   page-0 grid, in an equipment box or in the belt gives the tip lines
//!   of [`ItemTips`].
//! - Identify: a right press on a scroll or tome
//!   of identify in the grid uses it (C→S 0x20, which arms it on the
//!   server) and takes it as the used item (cursor state 6);
//!   the next left press on a grid item sends C→S 0x27 (target, used)
//!   and ends the state. A right press while the state is set cancels it.
//!
//! D2RUST's own, unverified: the right press is not settled (its flag is
//! set by the use handler, unwritten), the state is client-local, and the belt
//! is not hit-tested for the tip.

use super::{InvLayout, ItemsUi};
use crate::bridge::items::{self, mode, ItemView};
use crate::bridge::world::ClientWorld;
use crate::ui::geom::Point;
use crate::ui::item_tip::TipLine;
use crate::ui::layout::Screen;
use crate::ui::panel::ClientIntent;
use crate::ui::panels::{PanelOutput, UiFiles};
use d2_proto::client::UseItemAction;

/// The Horadric Cube's code (`items.txt` `box`).
pub fn is_cube(code: Option<[u8; 4]>) -> bool {
    matches!(code, Some(c) if &c == b"box ")
}

/// The identify scroll and tome codes (as the server's use effect,
/// `d2_sim::wiring::inventory::identify::IDENTIFY_CODES`).
pub fn is_identify(code: Option<[u8; 4]>) -> bool {
    matches!(code, Some(c) if &c == b"isc " || &c == b"ibk ")
}

/// The Town Portal scroll and tome codes (the server's use: item-use
/// entry 2, `d2_sim::wiring::inventory::item_use`).
pub fn is_portal(code: Option<[u8; 4]>) -> bool {
    matches!(code, Some(c) if &c == b"tsc " || &c == b"tbk ")
}

impl ItemsUi {
    /// The grid or equipped item under `at`, if any.
    pub fn item_at(
        &self,
        world: &ClientWorld,
        files: &UiFiles,
        layout: &InvLayout,
        at: Point,
    ) -> Option<ItemView> {
        let g = &layout.grid;
        let cell = (i32::from(g.cell_w), i32::from(g.cell_h));
        let all = items::local_items(world);
        if g.cell_w > 0 && g.cell_h > 0 && g.contains_mouse(at) {
            let (mc, mr) = g.mouse_cell(at);
            let (mc, mr) = (mc as i32, mr as i32);
            return all
                .into_iter()
                .filter(|i| i.mode == mode::STORED && i.page == 0)
                .find(|i| {
                    let (w, h) = self.art(files, i, cell).map_or((1, 1), |a| (a.w, a.h));
                    let (x, y) = (i32::from(i.x), i32::from(i.y));
                    (x..x + w).contains(&mc) && (y..y + h).contains(&mr)
                });
        }
        let loc = (1u8..=10).find(|&l| {
            let b = layout.equip[usize::from(l)];
            b.w > 0
                && b.h > 0
                && (b.left..b.left + b.w).contains(&at.x)
                && (b.top..b.top + b.h).contains(&at.y)
        })?;
        all.into_iter()
            .find(|i| i.mode == mode::BODY && i.body == loc)
    }

    /// The tool tip lines of the item under `at`; none with a cursor item
    /// (a cursor item clears the hover) or without tips.
    pub fn hover_lines(
        &self,
        world: &ClientWorld,
        files: &UiFiles,
        screen: &Screen,
        class: Option<u32>,
        at: Point,
    ) -> Vec<TipLine> {
        let (Some(tips), Some(layout)) = (&self.tips, self.layout(class, screen)) else {
            return Vec::new();
        };
        if items::cursor_item(world).is_some() {
            return Vec::new();
        }
        let Some(it) = self.item_at(world, files, &layout, at) else {
            return Vec::new();
        };
        // A grid item's tip is the hovered item of the
        // hover state, cleared by a use press.
        let hs = self.hover.get();
        if it.mode == mode::STORED && !(hs.in_grid && hs.item == Some(it.key.guid)) {
            return Vec::new();
        }
        // The fillers' properties are summed into
        // the item's own block (no separate filler lines).
        let me = crate::ui::item_tip_world::WorldUnit::local(world);
        let ctx = crate::ui::item_tip_world::hover_ctx(tips, world, me.as_ref(), &it);
        crate::ui::item_tip_world::model_bits(tips, world, it.key)
            .map_or_else(Vec::new, |b| tips.tip_lines_of(&b, &ctx))
    }

    /// The hover anchor of the grid item under
    /// `at`: x = left + cellW·c + (w·cellW)/2, top = top + cellH·r, bottom
    /// = top + cellH·(r + h). `None` for an equipped item (the box anchors
    /// are not specified: the caller falls back to the mouse point).
    pub fn hover_anchor(
        &self,
        world: &ClientWorld,
        files: &UiFiles,
        screen: &Screen,
        class: Option<u32>,
        at: Point,
    ) -> Option<crate::ui::item_tip::TipAnchor> {
        let layout = self.layout(class, screen)?;
        let it = self.item_at(world, files, &layout, at)?;
        if it.mode != mode::STORED {
            return None;
        }
        let g = &layout.grid;
        let cell = (i32::from(g.cell_w), i32::from(g.cell_h));
        let (w, h) = self.art(files, &it, cell).map_or((1, 1), |a| (a.w, a.h));
        Some(crate::ui::item_tip::TipAnchor::of_cell(
            (g.left, g.top),
            cell,
            (i32::from(it.x), i32::from(it.y)),
            (w, h),
        ))
    }

    /// A right press in the panel: an identify item in the grid is used (C→S 0x20, the server
    /// arms it) and becomes the used item (cursor state 6); while the state is set it cancels.
    /// A stored Horadric Cube (`box `) or Town Portal scroll / tome is used (C→S 0x20), which
    /// opens the cube (D2RUST's own; unverified).
    pub fn right_press(
        &self,
        world: &ClientWorld,
        files: &UiFiles,
        layout: &InvLayout,
        at: Point,
    ) -> Vec<PanelOutput> {
        if self.identify.get().is_some() {
            self.identify.set(None);
            return Vec::new();
        }
        if items::cursor_item(world).is_some() {
            return Vec::new();
        }
        let Some(it) = self.item_at(world, files, layout, at) else {
            return Vec::new();
        };
        if it.mode != mode::STORED {
            return Vec::new();
        }
        if !(is_identify(it.code) || is_cube(it.code) || is_portal(it.code)) {
            return Vec::new();
        }
        if is_identify(it.code) {
            // The use (C→S 0x20) arms the scroll or tome on the server
            // (item-use entry 1's first call, item flag 0x4); the target
            // click's 0x27 then identifies. Without it the 0x27 only armed
            // the item and nothing was identified.
            self.identify.set(Some(it.key.guid));
        }
        let (x, y) = world.local().and_then(|u| u.position).unwrap_or((0, 0));
        self.clear_hover();
        vec![PanelOutput::Intent(ClientIntent::from_message(
            &items::use_grid(it.key.guid, u32::from(x), u32::from(y)),
        ))]
    }

    /// The 0x27 intent of a [`GridMsg::TargetUsed`](super::super::inv_grid::GridMsg);
    /// ends the identify state.
    pub(super) fn target_used(&self, target: u32, used: u32) -> UseItemAction {
        self.identify.set(None);
        UseItemAction { target, used }
    }

    /// The identify state is on (the host may draw a cursor hint).
    pub fn identify_pending(&self) -> bool {
        self.identify.get().is_some()
    }
}
