//! Screen bins: per 32×32 bin of the view, the ordered indices of the items that touch it.
//!
//! The GPU compositor walks a pixel's bin list instead of every item in the frame. The bins are
//! built on the CPU, which also composes from them ([`super::cpu::compose_binned`]) to show that
//! binning changes nothing about the image.

use super::item::{DrawItem, FrameSource, MapTable};
use super::{Rect, SceneError, BIN_SIZE};

/// Item lists per bin, row-major, bins of [`BIN_SIZE`] pixels from the
/// view's top-left; the last column and row may be partial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bins {
    view: Rect,
    items: usize,
    cols: u32,
    rows: u32,
    lists: Vec<Vec<u32>>,
}

impl Bins {
    /// The view the bins cover.
    pub fn view(&self) -> Rect {
        self.view
    }

    /// Number of items in the list the bins were built from.
    pub fn item_count(&self) -> usize {
        self.items
    }

    /// Number of bin columns.
    pub fn cols(&self) -> u32 {
        self.cols
    }

    /// Number of bin rows.
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Item indices touching bin `(col, row)`, in list order.
    pub fn list(&self, col: u32, row: u32) -> &[u32] {
        &self.lists[(row * self.cols + col) as usize]
    }

    /// Screen rectangle of bin `(col, row)`, cut to the view.
    pub fn rect(&self, col: u32, row: u32) -> Rect {
        let x = i64::from(self.view.x) + i64::from(col * BIN_SIZE);
        let y = i64::from(self.view.y) + i64::from(row * BIN_SIZE);
        let w = BIN_SIZE.min(self.view.width - col * BIN_SIZE);
        let h = BIN_SIZE.min(self.view.height - row * BIN_SIZE);
        Rect::new(x as i32, y as i32, w, h)
    }
}

/// Bins `items` (already in draw order) over `view`. An item is listed in
/// every bin its drawable area (image ∩ clip ∩ view) overlaps, transparent
/// pixels included. Items are validated as for composing.
pub fn bin<F: FrameSource + ?Sized>(
    items: &[DrawItem],
    frames: &F,
    maps: &MapTable,
    view: Rect,
) -> Result<Bins, SceneError> {
    view.check_view()?;
    let cols = view.width.div_ceil(BIN_SIZE);
    let rows = view.height.div_ceil(BIN_SIZE);
    let mut lists = vec![Vec::new(); cols as usize * rows as usize];
    for (index, item) in items.iter().enumerate() {
        let (_, area) = item.resolve(frames, maps, &view).map_err(|e| e.at(index))?;
        let Some(area) = area else { continue };
        // Area lies inside the view, so offsets are in 0..view size.
        let x0 = (i64::from(area.x) - i64::from(view.x)) as u32;
        let y0 = (i64::from(area.y) - i64::from(view.y)) as u32;
        let (c0, c1) = (x0 / BIN_SIZE, (x0 + area.width - 1) / BIN_SIZE);
        let (r0, r1) = (y0 / BIN_SIZE, (y0 + area.height - 1) / BIN_SIZE);
        for r in r0..=r1 {
            for c in c0..=c1 {
                lists[(r * cols + c) as usize].push(index as u32);
            }
        }
    }
    Ok(Bins {
        view,
        items: items.len(),
        cols,
        rows,
        lists,
    })
}
