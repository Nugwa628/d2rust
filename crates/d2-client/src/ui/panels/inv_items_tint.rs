//! The tints under the inventory's items: the grid items and the
//! equipped items. Drawn before the item
//! graphics, so each graphic lands over its tint.
//!
//! Each tint is the UI rectangle primitive `(x, y, w, h, colour, 0)`:
//! draw mode 0, blend kind 2, `A2[256·d + c]` over the destination, the colour
//! the act palette's nearest entry of ([`ItemsUi::tint_colors`]). Read for the
//! requirement check: strength, dexterity and level from the tables
//! (requirement stat modifiers not applied). Not read: the shooter / quiver
//! term, the quest-item term and the
//! transmogrify cursor (state 8); with an item on the cursor nothing is
//! hovered, and [`ItemsUi::draw_placement_tint`] paints instead.

use super::super::draw::{RectRequest, UiDraw, UiDrawSink};
use super::super::geom::{Point, Rect};
use super::super::inv_grid::{
    equip_item_tint, grid_item_tint, hovered_tint, placement_tint, EquipItemFacts, GridItemFacts,
    GridRecord, Placement, Tint, Under,
};
use super::inv_items::{InvLayout, ItemsUi};
use super::UiFiles;
use crate::bridge::items::{self, mode, ItemView};
use crate::bridge::world::ClientWorld;

/// The tint's draw mode: blend kind 2, the 25 % alpha table.
pub const TINT_MODE: u8 = 0;

fn contains(r: &Rect, p: Point) -> bool {
    p.x >= r.x && p.x < r.x + i32::from(r.w) && p.y >= r.y && p.y < r.y + i32::from(r.h)
}

impl ItemsUi {
    /// The requirement check of `it` against the local player.
    fn requirements_fail(&self, world: &ClientWorld, it: &ItemView) -> bool {
        let (Some(me), Some(tips), Some(code)) = (world.local(), self.tips.as_ref(), it.code)
        else {
            return false;
        };
        !tips.can_use(code, me.stat(0), me.stat(2), me.stat(12))
    }

    fn grid_facts(&self, world: &ClientWorld, it: &ItemView) -> GridItemFacts {
        GridItemFacts {
            requirements_fail: self.requirements_fail(world, it),
            flag_4: it.flags & 0x4 != 0,
            identified: it.flags & 0x10 != 0,
            ..GridItemFacts::default()
        }
    }

    /// Paints the tints of the page-0 grid items and the equipped items
    /// of the local player (module doc). `mouse` is the cursor.
    pub fn draw_tints(
        &self,
        world: &ClientWorld,
        layout: &InvLayout,
        mouse: Point,
        out: &mut dyn UiDrawSink,
    ) {
        for it in items::local_items(world) {
            for d in self.item_tints(world, layout, mouse, &it) {
                out.push(d);
            }
        }
    }

    /// A grid item's tint over every cell of its footprint in
    /// grid `g` (the inventory page, the cube page: the grid draw draws
    /// them all), one rectangle per cell.
    pub(crate) fn grid_item_tints(
        &self,
        world: &ClientWorld,
        g: &GridRecord,
        mouse: Point,
        it: &ItemView,
    ) -> Vec<UiDraw> {
        let Some(colors) = self.tint_colors else {
            return Vec::new();
        };
        let hover_ok = items::cursor_item(world).is_none();
        let (w, h) = self.art.get(it.code.unwrap_or([0; 4])).map_or((1, 1), |a| {
            (i32::from(a.inv_w.max(1)), i32::from(a.inv_h.max(1)))
        });
        let (x, y, cw, ch) = g.cell(i32::from(it.x), i32::from(it.y));
        let r = Rect::new(x, y, (cw * w) as u16, (ch * h) as u16);
        // The hovered item of the hover state, under the mouse.
        let hs = self.hover.get();
        let hovered = hs.in_grid && hs.item == Some(it.key.guid);
        let t = if hover_ok && hovered && contains(&r, mouse) {
            hovered_tint(0, false)
        } else {
            grid_item_tint(&self.grid_facts(world, it))
        };
        let mut out = Vec::new();
        // Row by row, left to right (`a1-panel-cube` rows 5–8).
        for row in 0..h {
            for c in 0..w {
                out.push(UiDraw::Rect(RectRequest::sized(
                    x + cw * c,
                    y + ch * row,
                    cw,
                    ch,
                    colors[t as usize],
                    TINT_MODE,
                )));
            }
        }
        out
    }

    /// One item's tint rectangles: every cell of a grid item's footprint
    /// ("every cell of the w × h footprint is tinted": four 29 × 29 boxes
    /// for a 2 × 2 item),
    /// the body box of an equipped item.
    pub(crate) fn item_tints(
        &self,
        world: &ClientWorld,
        layout: &InvLayout,
        mouse: Point,
        it: &ItemView,
    ) -> Vec<UiDraw> {
        let Some(colors) = self.tint_colors else {
            return Vec::new();
        };
        let hover_ok = items::cursor_item(world).is_none();
        let paint = |t: Tint, r: Rect| {
            UiDraw::Rect(RectRequest::sized(
                r.x,
                r.y,
                i32::from(r.w),
                i32::from(r.h),
                colors[t as usize],
                TINT_MODE,
            ))
        };
        match it.mode {
            mode::STORED if it.page == 0 => self.grid_item_tints(world, &layout.grid, mouse, it),
            mode::BODY if (1..=10).contains(&it.body) => {
                let b = layout.equip[usize::from(it.body)];
                if b.w == 0 || b.h == 0 {
                    return Vec::new();
                }
                let r = Rect::new(b.left, b.top, b.w as u16, b.h as u16);
                let t = equip_item_tint(&EquipItemFacts {
                    hovered: hover_ok && contains(&r, mouse),
                    grid: self.grid_facts(world, it),
                    ..EquipItemFacts::default()
                });
                t.map(|t| paint(t, r)).into_iter().collect()
            }
            _ => Vec::new(),
        }
    }

    /// The placement tint for the cursor item over grid
    /// `g` (page `page`): drawn when the kept cursor cell is >= 0 and
    /// hover-in-grid is set, not with the mouse at or below `screen_h - 0x27`;
    /// tint 1 over the footprint when it fits, else 0 / 3 (the item
    /// under: the single overlap, or a cube of several), one rectangle per
    /// cell. PROVISIONAL: the mode 1 / 0x13 half-screen test (no tint
    /// with the mouse left of half the width) is the inventory panel's and not
    /// read here; open until a carried item over the left half with only the
    /// inventory open is checked.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_placement_tint(
        &self,
        world: &ClientWorld,
        files: &UiFiles,
        g: &GridRecord,
        page: u8,
        mouse: Point,
        screen_h: i32,
        out: &mut dyn UiDrawSink,
    ) {
        let (Some(colors), Some(cur)) = (self.tint_colors, items::cursor_item(world)) else {
            return;
        };
        let h = self.hover.get();
        let (c, r) = h.cursor_cell;
        if c < 0 || r < 0 || !h.in_grid || mouse.y >= screen_h - 0x27 || !g.contains_mouse(mouse) {
            return;
        }
        let cell = (i32::from(g.cell_w), i32::from(g.cell_h));
        let size = |i: &ItemView| {
            self.art.get(i.code.unwrap_or([0; 4])).map_or((1, 1), |a| {
                (i32::from(a.inv_w.max(1)), i32::from(a.inv_h.max(1)))
            })
        };
        let (w, hh) = self.art(files, &cur, cell).map_or((1, 1), |a| (a.w, a.h));
        let overlap: Vec<(ItemView, i32, i32)> = items::local_items(world)
            .into_iter()
            .filter(|i| i.mode == mode::STORED && i.page == page)
            .filter_map(|i| {
                let (iw, ih) = size(&i);
                let (x, y) = (i32::from(i.x), i32::from(i.y));
                (x < c + w && c < x + iw && y < r + hh && r < y + ih).then_some((i, iw, ih))
            })
            .collect();
        let fits =
            c + w <= i32::from(g.grid_x) && r + hh <= i32::from(g.grid_y) && overlap.is_empty();
        let n = overlap.len();
        let under = match n {
            0 => None,
            1 => Some(&overlap[0]),
            _ => overlap.iter().find(|(i, ..)| i.code == Some(*b"box ")),
        };
        let p = placement_tint(
            fits,
            under.map(|(i, ..)| Under {
                swap_ok: n == 1,
                is_cube: i.code == Some(*b"box "),
            }),
        );
        let (t, x, y, cw, ch, cols, rows) = match (p, under) {
            (Placement::Footprint(t), _) => {
                let (x, y, cw, ch) = g.cell(c, r);
                (t, x, y, cw, ch, w, hh)
            }
            (Placement::OverItem(t), Some((i, iw, ih))) => {
                let (x, y, cw, ch) = g.cell(i32::from(i.x), i32::from(i.y));
                (t, x, y, cw, ch, *iw, *ih)
            }
            _ => return,
        };
        // One rectangle per cell, row by row, left to right (measured
        // `w2-12-inv-tint-ama`: a 1 x 3 item draws three boxes down the
        // column).
        for row in 0..rows {
            for col in 0..cols {
                out.push(UiDraw::Rect(RectRequest::sized(
                    x + cw * col,
                    y + ch * row,
                    cw,
                    ch,
                    colors[t as usize],
                    TINT_MODE,
                )));
            }
        }
    }
}
