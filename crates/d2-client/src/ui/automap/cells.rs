//! The cell store: cells, the cel groups and the AVL tree each of a
//! layer's four cell sets is kept in.
//!
//! The tree shape matters: at equal (x, y) an insert compares the new cell
//! only with the nodes on its descent path, so whether a cell is
//! refused as a duplicate depends on which same-position cells it meets.
//! The tree is therefore a real AVL tree, not a sorted set.

use std::cmp::Ordering;

/// One automap cell. The pool's tree links and balance live in
/// [`CellTree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Cell {
    /// +0x00: 1 = came from the `.ma` file, 0 = added this session.
    pub saved: bool,
    /// +0x04: frame number in the block's cel file.
    pub cel: i16,
    /// +0x06, +0x08: automap units (screen pixels / 10).
    pub x: i16,
    pub y: i16,
}

impl Cell {
    /// A new (unsaved) cell.
    pub fn new(cel: i16, x: i16, y: i16) -> Self {
        Cell {
            saved: false,
            cel,
            x,
            y,
        }
    }
}

/// The 49 (cel, group) pairs of table.
pub const GROUP_PAIRS: [(i16, i8); 49] = [
    (0, 0),
    (1, 0),
    (2, 0),
    (3, 0),
    (6, 1),
    (7, 1),
    (8, 1),
    (11, 2),
    (12, 2),
    (13, 3),
    (14, 3),
    (20, 4),
    (38, 4),
    (21, 5),
    (39, 5),
    (46, 6),
    (47, 6),
    (48, 6),
    (49, 6),
    (51, 7),
    (52, 7),
    (53, 7),
    (54, 7),
    (60, 8),
    (70, 8),
    (61, 9),
    (71, 9),
    (120, 10),
    (169, 10),
    (171, 10),
    (121, 11),
    (170, 11),
    (172, 11),
    (257, 12),
    (258, 12),
    (259, 12),
    (266, 13),
    (267, 13),
    (337, 14),
    (338, 14),
    (472, 15),
    (473, 15),
    (474, 15),
    (475, 15),
    (520, 16),
    (521, 16),
    (522, 16),
    (533, 17),
    (534, 17),
];

/// Entries of the group array.
pub const GROUP_ENTRIES: usize = 2048;

/// g(c), the group of cel `c`, −1 when it has none. The array has
/// 2048 entries; the game indexes it unchecked, and no cel source
/// (`automap.txt`, `automapCel`, `AutoMap`, the `.ma` file after its
/// range check) gives a cel outside it, so an outside cel is read as −1.
pub fn group(cel: i16) -> i8 {
    if !(0..GROUP_ENTRIES as i16).contains(&cel) {
        return -1;
    }
    GROUP_PAIRS
        .iter()
        .find(|(c, _)| *c == cel)
        .map_or(-1, |&(_, g)| g)
}

/// Where a new cell goes relative to a node: keys compare y, then
/// x; at equal (x, y) the cell is a duplicate when g(new) = −1 or
/// g(new) = g(old), else it is ordered by cel number.
fn place(new: &Cell, old: &Cell) -> Option<Ordering> {
    match new.y.cmp(&old.y).then(new.x.cmp(&old.x)) {
        Ordering::Equal => {
            let g = group(new.cel);
            if g == -1 || g == group(old.cel) {
                None
            } else {
                Some(new.cel.cmp(&old.cel))
            }
        }
        o => Some(o),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Node {
    cell: Cell,
    /// +0x0A: height(right) − height(left), −1…1.
    balance: i8,
    left: Option<u32>,
    right: Option<u32>,
}

/// One cell tree of a layer (floors, walls, units, town art).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellTree {
    nodes: Vec<Node>,
    root: Option<u32>,
    /// Cells taken from the pool for this tree, refused ones included
    /// (the shared pool counter).
    allocated: usize,
}

impl CellTree {
    /// Inserts `c`. `false` when it is refused as a duplicate;
    /// its pool slot is still counted (edge case 1).
    pub fn insert(&mut self, c: Cell) -> bool {
        self.allocated += 1;
        let Some(root) = self.root else {
            self.root = Some(self.push(c));
            return true;
        };
        match self.insert_at(root, c) {
            None => false,
            Some((new_root, _)) => {
                self.root = Some(new_root);
                true
            }
        }
    }

    fn push(&mut self, c: Cell) -> u32 {
        self.nodes.push(Node {
            cell: c,
            balance: 0,
            left: None,
            right: None,
        });
        (self.nodes.len() - 1) as u32
    }

    /// Recursive AVL insert under `n`: the subtree's new root and whether
    /// its height grew, or `None` for a refused duplicate.
    fn insert_at(&mut self, n: u32, c: Cell) -> Option<(u32, bool)> {
        let ord = place(&c, &self.nodes[n as usize].cell)?;
        let left = ord == Ordering::Less;
        let child = if left {
            self.nodes[n as usize].left
        } else {
            self.nodes[n as usize].right
        };
        let (sub, grew) = match child {
            None => (self.push(c), true),
            Some(ch) => self.insert_at(ch, c)?,
        };
        let node = &mut self.nodes[n as usize];
        if left {
            node.left = Some(sub);
        } else {
            node.right = Some(sub);
        }
        if !grew {
            return Some((n, false));
        }
        node.balance += if left { -1 } else { 1 };
        match node.balance {
            0 => Some((n, false)),
            -1 | 1 => Some((n, true)),
            _ => Some((self.rebalance(n), false)),
        }
    }

    /// Rotations after an insert left `n` at balance ±2.
    fn rebalance(&mut self, n: u32) -> u32 {
        if self.nodes[n as usize].balance < 0 {
            let l = self.nodes[n as usize].left.expect("left-heavy node");
            if self.nodes[l as usize].balance <= 0 {
                self.rotate_right(n)
            } else {
                let nl = self.rotate_left(l);
                self.nodes[n as usize].left = Some(nl);
                self.rotate_right(n)
            }
        } else {
            let r = self.nodes[n as usize].right.expect("right-heavy node");
            if self.nodes[r as usize].balance >= 0 {
                self.rotate_left(n)
            } else {
                let nr = self.rotate_right(r);
                self.nodes[n as usize].right = Some(nr);
                self.rotate_left(n)
            }
        }
    }

    fn height_fix(&mut self, a: u32, b: u32, rotate_left: bool) {
        // Standard balance updates for a single rotation where `b` rises
        // over `a` (insert-time only: balances stay in −2…2).
        let (ba, bb) = (
            i32::from(self.nodes[a as usize].balance),
            i32::from(self.nodes[b as usize].balance),
        );
        let (na, nb) = if rotate_left {
            let na = ba - 1 - bb.max(0);
            let nb = bb - 1 + na.min(0);
            (na, nb)
        } else {
            let na = ba + 1 - bb.min(0);
            let nb = bb + 1 + na.max(0);
            (na, nb)
        };
        self.nodes[a as usize].balance = na as i8;
        self.nodes[b as usize].balance = nb as i8;
    }

    fn rotate_left(&mut self, a: u32) -> u32 {
        let b = self.nodes[a as usize]
            .right
            .expect("rotate_left needs a right child");
        self.nodes[a as usize].right = self.nodes[b as usize].left;
        self.nodes[b as usize].left = Some(a);
        self.height_fix(a, b, true);
        b
    }

    fn rotate_right(&mut self, a: u32) -> u32 {
        let b = self.nodes[a as usize]
            .left
            .expect("rotate_right needs a left child");
        self.nodes[a as usize].left = self.nodes[b as usize].right;
        self.nodes[b as usize].right = Some(a);
        self.height_fix(a, b, false);
        b
    }

    /// The cells in in-order (left, node, right): ascending (y, x, cel).
    pub fn in_order(&self) -> Vec<Cell> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack = Vec::new();
        let mut cur = self.root;
        while cur.is_some() || !stack.is_empty() {
            while let Some(n) = cur {
                stack.push(n);
                cur = self.nodes[n as usize].left;
            }
            let n = stack.pop().expect("non-empty stack");
            out.push(self.nodes[n as usize].cell);
            cur = self.nodes[n as usize].right;
        }
        out
    }

    /// Cells in the tree.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Pool cells taken, refused inserts included (edge case 1).
    pub fn allocated(&self) -> usize {
        self.allocated
    }

    /// Height of the tree (0 when empty); for the balance checks.
    pub fn height(&self) -> usize {
        fn h(t: &CellTree, n: Option<u32>) -> usize {
            n.map_or(0, |n| {
                let node = &t.nodes[n as usize];
                1 + h(t, node.left).max(h(t, node.right))
            })
        }
        h(self, self.root)
    }

    /// Whether every node's stored balance equals its subtree heights'
    /// difference and lies in −1…1 (the AVL invariant).
    pub fn balanced(&self) -> bool {
        fn walk(t: &CellTree, n: Option<u32>) -> Option<i32> {
            let Some(n) = n else { return Some(0) };
            let node = &t.nodes[n as usize];
            let l = walk(t, node.left)?;
            let r = walk(t, node.right)?;
            (r - l == i32::from(node.balance) && (r - l).abs() <= 1).then_some(1 + l.max(r))
        }
        walk(self, self.root).is_some()
    }
}

/// The four trees of a layer, in draw order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TreeKind {
    Floor,
    Wall,
    Unit,
    Town,
}

impl TreeKind {
    pub const ALL: [TreeKind; 4] = [
        TreeKind::Floor,
        TreeKind::Wall,
        TreeKind::Unit,
        TreeKind::Town,
    ];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// The cells of the current layer (only it is in memory).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayerCells {
    pub trees: [CellTree; 4],
}

impl LayerCells {
    pub fn tree(&self, k: TreeKind) -> &CellTree {
        &self.trees[k.index()]
    }

    pub fn tree_mut(&mut self, k: TreeKind) -> &mut CellTree {
        &mut self.trees[k.index()]
    }

    /// Every cell freed, the four roots:= 0.
    pub fn clear(&mut self) {
        *self = LayerCells::default();
    }
}
