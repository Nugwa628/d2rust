//! The item under the mouse for the shop's repair button: a page-0 grid
//! item or a worn one (D2RUST's own, unverified: the game's repair click
//! reads the same cell).

use super::{equip_loc, ItemsUi};
use crate::bridge::items::{self, mode, ItemView};
use crate::bridge::world::ClientWorld;
use crate::ui::geom::Point;

use super::super::super::inv_grid::GridRecord;

impl ItemsUi {
    /// The player's item under `at`: a page-0 grid item or a worn one.
    pub fn item_under(
        &self,
        world: &ClientWorld,
        grid: &GridRecord,
        layout: &super::InvLayout,
        at: Point,
    ) -> Option<ItemView> {
        let all = items::local_items(world);
        if grid.contains_mouse(at) {
            let (c, r) = grid.mouse_cell(at);
            let (c, r) = (c as i32, r as i32);
            return all.into_iter().find(|i| {
                let (w, h) = self.art.get(i.code.unwrap_or([0; 4])).map_or((1, 1), |a| {
                    (i32::from(a.inv_w.max(1)), i32::from(a.inv_h.max(1)))
                });
                let (x, y) = (i32::from(i.x), i32::from(i.y));
                i.mode == mode::STORED
                    && i.page == 0
                    && (x..x + w).contains(&c)
                    && (y..y + h).contains(&r)
            });
        }
        let loc = equip_loc(layout, at)?;
        all.into_iter()
            .find(|i| i.mode == mode::BODY && i.body == loc)
    }
}
