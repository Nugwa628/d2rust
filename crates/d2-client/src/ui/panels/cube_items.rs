//! The Horadric Cube grid (ui 0x1A, page 3): the local player's page-3 items
//! drawn with their inventory graphic and a left press on the grid sent as a
//! C→S item intent (the page-3 grid click is the inventory's, with page 3). The
//! server checks every move.
//!
//! Without `inventory.bin` rows the grid is record 9 / 25 as measured on
//! the install (the fallback; its corner point is settled, the rest
//! is not).

use super::super::draw::UiDrawSink;
use super::super::geom::Point;
use super::super::inv_grid::GridRecord;
use super::super::layout::Screen;
use super::inv_items::ItemsUi;
use super::{PanelOutput, UiFiles};
use crate::bridge::items::{self, mode};
use crate::bridge::world::ClientWorld;

/// The cube page.
pub const CUBE_PAGE: u8 = 3;

/// The `inventory.bin` record of the cube grid, "Transmogrify Box" (9),
/// +16 at 800 × 600.
pub fn cube_record(screen: &Screen) -> usize {
    if screen.res2() {
        9 + 16
    } else {
        9
    }
}

/// The install's `inventory.bin` record 9 / 25, as measured
/// (`prov_data_tables` checks it): (118, 205, 139, 253) at 640 × 480,
/// +(80, 60) at 800 × 600.
pub fn fallback_cube_grid(screen: &Screen) -> GridRecord {
    let (dx, dy) = (screen.sx(), -screen.sy() + screen.panel_dy());
    GridRecord {
        grid_x: 3,
        grid_y: 4,
        left: 118 + dx,
        right: 205 + dx,
        top: 139 + dy,
        bottom: 253 + dy,
        cell_w: 29,
        cell_h: 29,
    }
}

/// The cube panel's keep-open test: a local player whose mode is not 0x11
/// (dead). The exit flag is not in the model. A missing cube does not close the
/// panel.
pub fn cube_player_ok(world: &ClientWorld) -> bool {
    world.local().is_some_and(|u| u.mode != 0x11)
}

/// The cube item (code `box `, stored in the inventory) is in the model.
pub fn cube_present(world: &ClientWorld) -> bool {
    items::local_items(world)
        .iter()
        .any(|i| i.mode == mode::STORED && i.code == Some(*b"box "))
}

impl ItemsUi {
    /// The cube grid's record for this install.
    pub fn cube_grid(&self, screen: &Screen) -> GridRecord {
        match &self.layouts {
            Some(l) => l
                .get(cube_record(screen))
                .map_or_else(|| fallback_cube_grid(screen), |l| l.grid),
            None => fallback_cube_grid(screen),
        }
    }

    /// The local player's page-3 items (stored mode), graphics drawn with
    /// the frame's top-left at the cell.
    pub fn draw_cube(
        &self,
        world: &ClientWorld,
        files: &UiFiles,
        g: &GridRecord,
        mouse: Point,
        out: &mut dyn UiDrawSink,
    ) {
        // `a1-panel-cube` rows 5–9: tints, then the item.
        let list: Vec<_> = items::local_items(world)
            .into_iter()
            .filter(|it| it.mode == mode::STORED && it.page == CUBE_PAGE)
            .collect();
        self.draw_grid_items(world, files, g, &list, mouse, out);
    }

    /// Left mouse down on the cube grid: the page-3 grid click.
    pub fn press_cube(
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
        self.grid_press(world, files, g, cursor.as_ref(), at, CUBE_PAGE)
            .map(PanelOutput::Intent)
            .into_iter()
            .collect()
    }
}
