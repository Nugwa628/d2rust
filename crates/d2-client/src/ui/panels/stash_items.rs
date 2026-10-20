//! The stash grid (ui 0x19, page 4): the local player's page-4 items drawn with
//! their inventory graphic and a left press on the grid sent as a C→S item
//! intent (the page-4 grid click is the inventory's; with page 4 and inventory
//! mode 0x0C). Plain decisions over the client model; the server checks every
//! move.
//!
//! Without `inventory.bin` rows the grid is the install's record 12 / 28
//! (expansion, 6 × 8) or 8 / 24 (classic, 6 × 4), as measured;
//! the grid corner is settled (its inventory-half
//! point is not).

use super::super::draw::UiDrawSink;
use super::super::geom::Point;
use super::super::inv_grid::GridRecord;
use super::super::layout::Screen;
use super::inv_items::ItemsUi;
use super::{PanelOutput, UiFiles};
use crate::bridge::items::{self, mode};
use crate::bridge::world::ClientWorld;

/// The stash page.
pub const STASH_PAGE: u8 = 4;

/// The `inventory.bin` record of the stash grid: 8 (classic) or 12
/// (expansion), +16 at 800 × 600.
pub fn stash_record(expansion: bool, screen: &Screen) -> usize {
    let r = if expansion { 12 } else { 8 };
    if screen.res2() {
        r + 16
    } else {
        r
    }
}

/// The install's `inventory.bin` rows, as measured
/// (`prov_data_tables` checks them): the expansion stash, record 12 / 28,
/// is (74, 244, 82, 313) at 640 × 480; the classic stash, record 8 / 24,
/// (74, 244, 273, 386); both +(80, 60) at 800 × 600.
pub fn fallback_stash_grid(expansion: bool, screen: &Screen) -> GridRecord {
    let (dx, dy) = (screen.sx(), -screen.sy() + screen.panel_dy());
    let (rows, top, bottom): (u8, i32, i32) = if expansion {
        (8, 82, 313)
    } else {
        (4, 273, 386)
    };
    GridRecord {
        grid_x: 6,
        grid_y: rows,
        left: 74 + dx,
        right: 74 + dx + 170,
        top: top + dy,
        bottom: bottom + dy,
        cell_w: 29,
        cell_h: 29,
    }
}

impl ItemsUi {
    /// The stash grid's record for this install.
    pub fn stash_grid(&self, expansion: bool, screen: &Screen) -> GridRecord {
        match &self.layouts {
            Some(l) => l
                .get(stash_record(expansion, screen))
                .map_or_else(|| fallback_stash_grid(expansion, screen), |l| l.grid),
            None => fallback_stash_grid(expansion, screen),
        }
    }

    /// The local player's page-4 items (stored mode), graphics drawn with
    /// the frame's top-left at the cell.
    pub fn draw_stash(
        &self,
        world: &ClientWorld,
        files: &UiFiles,
        g: &GridRecord,
        out: &mut dyn UiDrawSink,
    ) {
        let cell = (i32::from(g.cell_w), i32::from(g.cell_h));
        for it in items::local_items(world) {
            if it.mode != mode::STORED || it.page != STASH_PAGE {
                continue;
            }
            let Some(a) = self.art(files, &it, cell) else {
                continue;
            };
            let (x, y, _, _) = g.cell(i32::from(it.x), i32::from(it.y));
            out.push(self.item_cel(world, &it, a.file, x, y + a.gh));
        }
    }

    /// Left mouse down on the stash grid: the page-4 grid click.
    pub fn press_stash(
        &self,
        world: &ClientWorld,
        files: &UiFiles,
        g: &GridRecord,
        at: Point,
    ) -> Vec<PanelOutput> {
        if g.cell_w == 0 || g.cell_h == 0 || !g.contains_mouse(at) {
            return Vec::new();
        }
        let cursor = items::cursor_item(world);
        self.grid_press(world, files, g, cursor.as_ref(), at, STASH_PAGE)
            .map(PanelOutput::Intent)
            .into_iter()
            .collect()
    }
}
