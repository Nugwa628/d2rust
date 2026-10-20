//! Grid record by page, the fit test, place at a position,
//! the free-position search and its weight and the item
//! placement into a page.

use super::{
    cmd, grid_id, iflag, mode, node, page, targeting_reset, Grid, InvTables, InvWorld, Inventory,
    UnitKind, BODY_GRID, NO_GUID,
};
use crate::units::UnitId;

/// Player class → `inventory.bin` record (7 pairs).
pub const CLASS_RECORDS: [(u8, usize); 7] =
    [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 14), (6, 15)];
/// Monster record.
pub const MONSTER_RECORD: usize = 5;
/// Object classes with a grid record.
pub const OBJECT_RECORDS: [(u32, usize); 2] = [(0x152, 10), (0x153, 11)];

/// Grid record by page: the `inventory.bin` record
/// of an owner's page; none = −1.
pub fn grid_record(owner: UnitKind, pg: u8, expansion: bool) -> Option<usize> {
    match owner {
        UnitKind::Player { class } => match pg {
            1 => Some(6),
            2 => Some(7),
            3 => Some(9),
            4 => Some(if expansion { 12 } else { 8 }),
            _ => CLASS_RECORDS
                .iter()
                .find(|&&(c, _)| c == class)
                .map(|&(_, r)| r),
        },
        UnitKind::Monster { .. } => Some(MONSTER_RECORD),
        UnitKind::Object { class } => OBJECT_RECORDS
            .iter()
            .find(|&&(c, _)| c == class)
            .map(|&(_, r)| r),
        // Missiles, items, tiles (types 3–5): no record (socket
        // fillers join an item's inventory through).
        UnitKind::Item | UnitKind::Other => None,
    }
}

/// The page grid size gridX × gridY of an owner's page.
pub fn page_grid_size(t: &InvTables, owner: UnitKind, pg: u8, expansion: bool) -> Option<(u8, u8)> {
    let r = t.grids.get(grid_record(owner, pg, expansion)?)?;
    Some((r.grid_x, r.grid_y))
}

/// Fit test: every cell of the w × h rectangle at
/// (x, y) is empty. Callers check the bounds first. The loops run from x
/// to x + w with a signed 32-bit `<` and a wrapping end: an end
/// that wraps past 2^31 makes them run zero times (the item fits).
pub fn fits(g: &Grid, x: i32, y: i32, w: u8, h: u8) -> bool {
    (y..y.wrapping_add(i32::from(h)))
        .all(|yy| (x..x.wrapping_add(i32::from(w))).all(|xx| g.cell(xx, yy).is_none()))
}

/// Bounds of [`fits`] and placement: x < 0 or x + w >
/// width fails, the same for y, in signed 32-bit arithmetic with wrap: an
/// x + w (or y + h) that wraps past 2^31 is negative and passes (kept on
/// purpose: 0x18 with x = 0x7FFFFFFF places a 1-wide item).
pub fn in_bounds(g: &Grid, x: i32, y: i32, w: u8, h: u8) -> bool {
    x >= 0
        && y >= 0
        && x.wrapping_add(i32::from(w)) <= i32::from(g.width)
        && y.wrapping_add(i32::from(h)) <= i32::from(g.height)
}

/// Weight of a fitting candidate: occupied cells just
/// outside each side, a side on the grid edge counting as fully occupied;
/// a total ≥ 2 × (w + h) is 255.
pub fn weight(g: &Grid, x: i32, y: i32, w: u8, h: u8) -> u8 {
    let (wi, hi) = (i32::from(w), i32::from(h));
    let occ = |xx: i32, yy: i32| u32::from(g.cell(xx, yy).is_some());
    let col = |xx: i32| (y..y + hi).map(|yy| occ(xx, yy)).sum::<u32>();
    let row = |yy: i32| (x..x + wi).map(|xx| occ(xx, yy)).sum::<u32>();
    let left = if x > 0 { col(x - 1) } else { u32::from(h) };
    let right = if x + wi < i32::from(g.width) {
        col(x + wi)
    } else {
        u32::from(h)
    };
    let top = if y > 0 { row(y - 1) } else { u32::from(w) };
    let bottom = if y + hi < i32::from(g.height) {
        row(y + hi)
    } else {
        u32::from(w)
    };
    let total = left + right + top + bottom;
    if total >= 2 * (u32::from(w) + u32::from(h)) {
        255
    } else {
        // The total stays below 2 × (w + h) ≤ 2 × 510 here; the u8 of the
        // base game keeps its low byte.
        total as u8
    }
}

fn candidate(g: &Grid, x: i32, y: i32, w: u8, h: u8) -> bool {
    g.cell(x, y).is_none() && in_bounds(g, x, y, w, h) && fits(g, x, y, w, h)
}

/// Free-position search on a grid: the four strategies by item
/// height and owner. Returns the top-left cell.
pub fn search(g: &Grid, w: u8, h: u8, player: bool) -> Option<(i32, i32)> {
    if w == 0 || h == 0 {
        return None;
    }
    let (gw, gh) = (i32::from(g.width), i32::from(g.height));
    let mut order: Vec<(i32, i32)> = Vec::new();
    match (h == 1, player) {
        // X from width−1 down, then y from height−1 down.
        (true, true) => {
            for x in (0..gw).rev() {
                for y in (0..gh).rev() {
                    order.push((x, y));
                }
            }
        }
        // X from width−1 down, then y from 0 up.
        (true, false) => {
            for x in (0..gw).rev() {
                for y in 0..gh {
                    order.push((x, y));
                }
            }
        }
        // Y from 0 up, then x from 0 up.
        (false, true) => {
            for y in 0..gh {
                for x in 0..gw {
                    order.push((x, y));
                }
            }
        }
        // X from 0 up, then y from 0 up.
        (false, false) => {
            for x in 0..gw {
                for y in 0..gh {
                    order.push((x, y));
                }
            }
        }
    }
    let mut cands = order.into_iter().filter(|&(x, y)| candidate(g, x, y, w, h));
    if !player {
        return cands.next();
    }
    let mut best: Option<(i32, i32)> = None;
    let mut best_w = 0u8;
    for (x, y) in cands {
        let wt = weight(g, x, y, w, h);
        if wt > best_w {
            best_w = wt;
            best = Some((x, y));
            if wt == 255 {
                break;
            }
        }
    }
    // Succeeds only if the best weight > 0 (edge case 9).
    best.filter(|_| best_w > 0)
}

/// Place at a position, the shared core: item `item`
/// of `iw` × `ih` cells at (x, y) of grid `g` (created gw × gh).
#[allow(clippy::too_many_arguments)]
pub fn place_in_grid<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    item: UnitId,
    g: usize,
    x: i32,
    y: i32,
    (iw, ih): (u8, u8),
    (gw, gh): (u8, u8),
) -> bool {
    let Some(grid) = inv.grid_or_create(g, gw, gh) else {
        return false;
    };
    // Grids 0 and 1 skip the bound test (1 × 1; callers check the slot).
    if (g >= grid_id::PAGE && !in_bounds(grid, x, y, iw, ih)) || !fits(grid, x, y, iw, ih) {
        return false;
    }
    let Some(d) = w.item(item).copied() else {
        return false;
    };
    if d.mode == mode::GROUND {
        w.remove_from_room(item);
    }
    match d.inv {
        // The cursor item belongs to this inventory (a state
        // whose +0x5C was not written is treated alike).
        _ if inv.cursor() == Some(item) => {
            inv.unlink(w, item);
        }
        Some(o) if o == inv.owner => {
            inv.unlink(w, item);
        }
        Some(o) => w.unlink_from(o, item),
        None => {}
    }
    inv.link(w, item, Some(g));
    if let Some(Some(grid)) = inv.grids.get_mut(g) {
        grid.set_rect(x, y, iw, ih, Some(item));
    }
    inv.count = inv.count.wrapping_add(1);
    let owner_guid = if inv.owner_kind.is_player() {
        inv.owner_guid
    } else {
        NO_GUID
    };
    if let Some(d) = w.item_mut(item) {
        d.node_grid = (g + 1) as u8;
        d.x = x;
        d.y = y;
        d.owner_guid = owner_guid;
        if g >= grid_id::PAGE {
            d.page = (g - grid_id::PAGE) as u8;
        }
        d.node_kind = match g {
            grid_id::BODY if x <= 10 => node::BODY,
            grid_id::BODY => node::SWAP,
            grid_id::BELT => node::BELT,
            _ => node::PAGE,
        };
    }
    true
}

/// Place at a position of a page:
/// grid = page + 2; items with invwidth or invheight 0 never place.
pub fn place_at_page<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    t: &InvTables,
    item: UnitId,
    pg: u8,
    x: i32,
    y: i32,
) -> bool {
    let Some(size) = w.item(item).and_then(|d| t.size(d.record)) else {
        return false;
    };
    if size.0 == 0 || size.1 == 0 {
        return false;
    }
    let Some(gsize) = page_grid_size(t, inv.owner_kind, pg, w.expansion()) else {
        return false;
    };
    place_in_grid(
        inv,
        w,
        item,
        grid_id::PAGE + usize::from(pg),
        x,
        y,
        size,
        gsize,
    )
}

/// Put at body location `loc`: on grid
/// 0, the item as 1 × 1 at x = loc.
pub fn place_at_body<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    item: UnitId,
    loc: u8,
) -> bool {
    place_in_grid(
        inv,
        w,
        item,
        grid_id::BODY,
        i32::from(loc),
        0,
        (1, 1),
        BODY_GRID,
    )
}

/// Free-position search for `item` on page `pg`.
pub fn find_free_position<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &W,
    t: &InvTables,
    item: UnitId,
    pg: u8,
) -> Option<(i32, i32)> {
    let (iw, ih) = w.item(item).and_then(|d| t.size(d.record))?;
    if iw == 0 || ih == 0 {
        return None;
    }
    let (gw, gh) = page_grid_size(t, inv.owner_kind, pg, w.expansion())?;
    let player = inv.owner_kind.is_player();
    let g = inv.grid_or_create(grid_id::PAGE + usize::from(pg), gw, gh)?;
    search(g, iw, ih, player)
}

/// Item placement into a page, steps 1–9: the
/// cursor item `item` (none = missing) into the page its item data names,
/// at (x, y) or a free position. Returns the result (1 = true).
#[allow(clippy::too_many_arguments)]
pub fn place_in_page<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    t: &InvTables,
    item: Option<UnitId>,
    x: i32,
    y: i32,
    find_free: bool,
    send: bool,
) -> bool {
    targeting_reset(inv, w);
    place_in_page_from_cursor(inv, w, t, item, x, y, find_free, send)
}

/// [`place_in_page`] without the targeting reset (for the move paths that
/// place from the cursor and keep the target).
#[allow(clippy::too_many_arguments)]
pub fn place_in_page_from_cursor<W: InvWorld + ?Sized>(
    inv: &mut Inventory,
    w: &mut W,
    t: &InvTables,
    item: Option<UnitId>,
    x: i32,
    y: i32,
    find_free: bool,
    send: bool,
) -> bool {
    // Step 2.
    let Some(item) = item else { return false };
    let Some(d) = w.item(item).copied() else {
        return false;
    };
    if d.mode != mode::CURSOR {
        return false;
    }
    // Step 3.
    let pg = d.page;
    let player = inv.owner_kind.is_player();
    let keep_cursor = player && pg == page::TRADE1;
    // Step 4.
    let placed = if find_free {
        match find_free_position(inv, w, t, item, pg) {
            Some((fx, fy)) => place_at_page(inv, w, t, item, pg, fx, fy),
            None => false,
        }
    } else {
        place_at_page(inv, w, t, item, pg, x.max(0), y.max(0))
    };
    if !placed {
        return false;
    }
    // Step 5. A failure returns 0 with nothing undone: the item stays
    // placed by step 4 (the placement's unlink cleared the cursor) in
    // mode 4 (a base-game bug, kept on purpose).
    if !w.link_check(inv.owner, item, 1) {
        return false;
    }
    // Step 6.
    if pg != page::STASH {
        w.charm_relink(inv.owner, item);
    }
    w.clear_targetable(item);
    if w.active_item(inv.owner, item) {
        w.stat_refresh(inv.owner);
    }
    // Step 7.
    if !keep_cursor {
        inv.put_cursor(w, None);
    }
    let filled = w.socket_filled(item);
    let mut guid = d.guid;
    if let Some(d) = w.item_mut(item) {
        d.mode = mode::STORED;
        guid = d.guid;
        // Step 8.
        if send {
            d.cmd_flags |= cmd::PUT_IN_CONTAINER;
            if filled {
                d.flags |= iflag::CHANGED;
            }
            d.flags &= !iflag::F4000;
        }
    }
    if send {
        w.owner_refresh(inv.owner);
        inv.push_update(guid);
    }
    // Page-2 trade hook: after step 8, before step 9.
    if player && pg == page::TRADE2 {
        w.trade_hook(inv.owner, item);
    }
    // Step 9.
    if w.active_item(inv.owner, item) {
        w.inventory_pass(inv.owner);
        w.owner_refresh(inv.owner);
    }
    true
}
