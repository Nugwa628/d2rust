//! The cell draw pass: blocks and clip windows, the pruned
//! in-order walk, the clip rectangle and the fade draw mode.

use super::cells::{CellTree, LayerCells, TreeKind};
use super::options::{CelFile, OptionStore, Options};
use super::town::TownKind;
use super::view::{Bounds, FrameFacts, View};

/// One draw of the automap pass, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomapDraw {
    /// CelDrawClipped of frame `cel` of `file` at (x, y),
    /// clipped to `clip`, draw mode `mode`.
    Cel {
        file: CelFile,
        cel: i16,
        x: i32,
        y: i32,
        clip: Bounds,
        mode: u8,
    },
    /// An opaque line in palette index
    /// `color`.
    Line {
        from: (i32, i32),
        to: (i32, i32),
        color: u8,
    },
    /// Text `text` in font `font`, colour `color`, centred on `x`
    /// (`align` [`TextAlign::Centre`]) or ending at `x` (right-aligned),
    /// with `y` the line's top or the header line's y.
    Text {
        text: Label,
        font: u16,
        color: u16,
        x: i32,
        y: i32,
        align: TextAlign,
    },
}

/// The text of a [`AutomapDraw::Text`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Label {
    /// UTF-16 code units.
    Text(Vec<u16>),
    /// A string-table id the sink looks up.
    String(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextAlign {
    /// Centred on x, top at y.
    Centre,
    /// Right edge at x (x = W − width − 16).
    Right,
}

/// One block of the cell pass: the tree, its file, the clip window and (dx, dy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Block {
    pub tree: TreeKind,
    pub file: CelFile,
    pub window: Bounds,
    pub d: (i32, i32),
}

/// The four blocks in draw order (floors, walls, units, town).
pub fn blocks(view: &View, town: TownKind, f: &FrameFacts) -> [Block; 4] {
    let (w, hp) = (f.width, f.play_height());
    let (mx, my) = view.mini_origin;
    let window = |a: i32, b: i32| {
        if view.mini {
            Bounds {
                left: mx - a,
                top: my - b,
                right: w / 3 + mx + a,
                bottom: hp / 3 + my + b,
            }
        } else {
            Bounds {
                left: -a,
                top: -b,
                right: w + a,
                bottom: hp + b,
            }
        }
    };
    let maxi = |tree| Block {
        tree,
        file: CelFile::MaxiMap,
        window: if view.mini {
            Bounds {
                left: mx - 8,
                top: my - 16,
                right: w / 3 + mx + 8,
                bottom: hp / 3 + my + 16,
            }
        } else {
            Bounds {
                left: -16,
                top: -32,
                right: w + 16,
                bottom: hp + 32,
            }
        },
        d: if view.mini { (4, 8) } else { (8, 16) },
    };
    let town_block = match town {
        TownKind::None => maxi(TreeKind::Town),
        k => {
            let (a, b, d) = match k {
                TownKind::LutGholein => (160, 100, (80, 50)),
                TownKind::Pandemonium => (136, 90, (68, 45)),
                _ => (180, 170, (90, 85)),
            };
            Block {
                tree: TreeKind::Town,
                file: CelFile::of_town(k),
                window: window(a, b),
                d,
            }
        }
    };
    [
        maxi(TreeKind::Floor),
        maxi(TreeKind::Wall),
        maxi(TreeKind::Unit),
        town_block,
    ]
}

/// The clip rectangle of every cel draw.
pub fn clip_rect(view: &View, opts: &Options, f: &FrameFacts) -> Bounds {
    if !view.mini {
        return Bounds {
            left: 0,
            top: 0,
            right: f.width - 1,
            bottom: f.height - 1,
        };
    }
    let (x0, y0) = if f.mini_centred() {
        ((f.width - 279) / 2, 57)
    } else if !opts.left {
        (f.width - 281, 57)
    } else {
        (0, (if f.mini_down { 96 } else { 0 }) - 21)
    };
    Bounds {
        left: x0,
        top: y0,
        right: x0 + 279,
        bottom: y0 + 225,
    }
}

/// The fade value v the pass draws with. In mini with v = 1 it becomes
/// the fade latch ≠ 0 ? 0: 2 and is stored.
pub fn pass_fade(view: &View, opts: &mut Options, store: &mut dyn OptionStore) -> u32 {
    if view.mini && opts.fade == 1 {
        let v = if opts.fade_latch != 0 { 0 } else { 2 };
        opts.set_fade(v, store);
    }
    opts.fade
}

/// What the mode of one cel reads besides the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FadeFacts {
    /// The pass's fade v ([`pass_fade`]).
    pub v: u32,
    pub open_mode: u8,
    /// The local player's byte +0x18 (fade 3).
    pub player_byte_18: u8,
}

/// Draw mode m of a cel of `block` at screen (X, Y).
pub fn draw_mode(
    view: &View,
    block: &Block,
    at: (i32, i32),
    fade: FadeFacts,
    f: &FrameFacts,
) -> u8 {
    match fade.v {
        1 if !view.mini && block.file == CelFile::MaxiMap => {
            let (w2, hp2) = (f.width / 2, f.play_height() / 2);
            let s2 = match fade.open_mode {
                1 => -f.panel_shift(),
                2 => f.panel_shift(),
                _ => 0,
            };
            let (sx, sy) = (at.0 + block.d.0, at.1 + block.d.1);
            let inside =
                w2 + s2 - 140 <= sx && sx <= w2 + s2 + 140 && hp2 - 150 <= sy && sy <= hp2 + 130;
            if !inside {
                return 5;
            }
            let (a, b) = ((w2 - sx + s2).abs(), (hp2 - 10 - sy).abs());
            let e = (2 * a.max(b) + a.min(b)) / 2;
            match e {
                e if e < 50 => 0,
                e if e < 100 => 1,
                e if e < 150 => 2,
                _ => 5,
            }
        }
        2 => 1,
        // V = 3 honours the act byte (unit +0x18) in
        // both sizes: 0 or 2 -> 1 (mini) / 2 (full); else 5.
        3 if matches!(fade.player_byte_18, 0 | 2) => {
            if view.mini {
                1
            } else {
                2
            }
        }
        // v = 1 on a town file (edge case 4), v = 3 otherwise, other v.
        _ => 5,
    }
}

/// The walk of one tree: in-order, cells with y min ≤ Y ≤ y max
/// and x min ≤ X ≤ x max, stopping at the first Y > y max. Y is
/// non-decreasing in the tree order (y·10 / div is monotonic), so the
/// game's pruning only skips cells this filter rejects.
pub fn walk(tree: &CellTree, view: &View, a: (i32, i32), window: &Bounds) -> Vec<(i16, i32, i32)> {
    let mut out = Vec::new();
    for c in tree.in_order() {
        let (x, y) = view.cell_screen(c.x, c.y, a);
        if y > window.bottom {
            break;
        }
        if window.top <= y && window.left <= x && x <= window.right {
            out.push((c.cel, x, y));
        }
    }
    out
}

/// The facts of one cell pass.
#[derive(Debug, Clone, Copy)]
pub struct CellPass<'a> {
    pub view: &'a View,
    pub opts: &'a Options,
    /// (Ax, Ay), the drawing origin.
    pub a: (i32, i32),
    pub fade: FadeFacts,
    pub frame: &'a FrameFacts,
}

/// R2–r4: every cel draw of the current layer's trees.
pub fn draw_cells(cells: &LayerCells, town: TownKind, p: &CellPass, out: &mut Vec<AutomapDraw>) {
    let clip = clip_rect(p.view, p.opts, p.frame);
    for block in blocks(p.view, town, p.frame) {
        for (cel, x, y) in walk(cells.tree(block.tree), p.view, p.a, &block.window) {
            out.push(AutomapDraw::Cel {
                file: block.file,
                cel,
                x,
                y,
                clip,
                mode: draw_mode(p.view, &block, (x, y), p.fade, p.frame),
            });
        }
    }
}
