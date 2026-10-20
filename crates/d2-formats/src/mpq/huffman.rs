//! The MPQ adaptive Huffman decoder (and test encoder) for MPQ sectors.
//!
//! [`decompress`] decodes the Huffman stage of a compressed sector: the first byte picks one
//! of nine weight tables (table 0 is adaptive), and the tree is updated after every symbol
//! so encoder and decoder stay in step. `Archive` calls it from the sector decompressor;
//! the weight tables are in `tables`. `compress` is the inverse, used by the test-only
//! writer and by tests.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::bits::BitReader;
use super::tables::HUFFMAN_WEIGHTS;
use super::CodecError;

/// Symbol that ends the stream.
pub(super) const SYM_END: u16 = 0x100;
/// Symbol followed by a literal byte that has no leaf yet.
pub(super) const SYM_ESCAPE: u16 = 0x101;

/// Output bytes per input byte, at most: every symbol takes at least one
/// bit. Bounds the preallocation, since `max_out` is untrusted.
const MAX_RATIO: usize = 8;

/// Slots of the leader cache (a power of two).
const LEADER_SLOTS: usize = 256;
const NO_NODE: u32 = u32::MAX;

fn err(reason: &'static str) -> CodecError {
    CodecError {
        codec: "huffman",
        reason,
    }
}

#[derive(Clone)]
struct Node {
    weight: u32,
    /// `Some(symbol)` for a leaf, `None` for a branch.
    symbol: Option<u16>,
    /// Branch children: [child0, child1].
    child: [usize; 2],
    parent: Option<usize>,
    /// Neighbours in the weight-ordered list (head = heaviest).
    prev: Option<usize>,
    next: Option<usize>,
}

/// The tree plus its weight-ordered node list. Cloned from a per-table
/// template for every stream.
#[derive(Clone)]
struct Tree {
    nodes: Vec<Node>,
    head: usize,
    tail: usize,
    /// Leader cache: slot `w % LEADER_SLOTS` holds a node that may be the
    /// first node in the list (closest to the head) with weight `w`. Only
    /// a hint: [`Tree::leader`] checks it against the list and falls back
    /// to a walk, so a stale or colliding entry never changes a result.
    leader: [u32; LEADER_SLOTS],
}

impl Tree {
    /// Builds the starting tree of a weight table: a leaf per symbol with a non-zero
    /// weight (in symbol order), the end and escape leaves at weight 1, then, walking
    /// up from the tail, a parent branch for each pair of neighbours.
    fn build(weights: &[u8; 256]) -> Tree {
        let mut b = Builder::default();
        for (sym, &w) in weights.iter().enumerate() {
            if w != 0 {
                b.insert_leaf(sym as u16, u32::from(w));
            }
        }
        b.insert_leaf(SYM_END, 1);
        b.insert_leaf(SYM_ESCAPE, 1);

        // Inserting branches can move the head, so re-read it every pass.
        let mut c = b.tail.expect("tree has leaves");
        while Some(c) != b.head {
            let a = c;
            let bb = b.nodes[a].prev.expect("a is not the head");
            let w = b.nodes[a].weight + b.nodes[bb].weight;
            let m = b.push(Node {
                weight: w,
                symbol: None,
                child: [a, bb],
                parent: None,
                prev: None,
                next: None,
            });
            b.nodes[a].parent = Some(m);
            b.nodes[bb].parent = Some(m);
            b.insert(m);
            match b.nodes[bb].prev {
                Some(p) => c = p,
                None => break,
            }
        }

        let head = b.head.expect("tree has nodes");
        let tail = b.tail.expect("tree has nodes");
        let mut tree = Tree {
            nodes: b.nodes,
            head,
            tail,
            leader: [NO_NODE; LEADER_SLOTS],
        };
        let mut cur = Some(head);
        while let Some(i) = cur {
            tree.note_leader(i);
            cur = tree.nodes[i].next;
        }
        tree
    }

    /// Whether `c` is the first node in the list with weight `w`. The list
    /// is non-increasing, so that node is the only one of weight `w` whose
    /// predecessor (if any) has another weight.
    fn is_leader(&self, c: usize, w: u32) -> bool {
        self.nodes.get(c).is_some_and(|node| {
            node.weight == w && node.prev.is_none_or(|p| self.nodes[p].weight != w)
        })
    }

    /// Records `c` in the cache if it is the first node of its weight.
    fn note_leader(&mut self, c: usize) {
        let w = self.nodes[c].weight;
        if self.is_leader(c, w) {
            self.leader[w as usize % LEADER_SLOTS] = c as u32;
        }
    }

    /// The first node in the list with `n`'s weight (the node `n` trades places with
    /// before its weight goes up):
    /// the cached node if it checks out, else a walk back from `n` through
    /// its weight block.
    fn leader(&mut self, n: usize) -> usize {
        let w = self.nodes[n].weight;
        let slot = w as usize % LEADER_SLOTS;
        let c = self.leader[slot] as usize;
        if self.is_leader(c, w) {
            return c;
        }
        let mut c = n;
        while let Some(p) = self.nodes[c].prev {
            if self.nodes[p].weight != w {
                break;
            }
            c = p;
        }
        self.leader[slot] = c as u32;
        c
    }

    fn root(&self) -> usize {
        self.head
    }

    /// Exchanges the list positions of `x` and `y`.
    fn swap_list(&mut self, x: usize, y: usize) {
        if self.nodes[x].next == Some(y) {
            self.move_before(y, x);
        } else if self.nodes[y].next == Some(x) {
            self.move_before(x, y);
        } else {
            let (xp, xn) = (self.nodes[x].prev, self.nodes[x].next);
            let (yp, yn) = (self.nodes[y].prev, self.nodes[y].next);
            self.link(xp, Some(y));
            self.link(Some(y), xn);
            self.link(yp, Some(x));
            self.link(Some(x), yn);
        }
    }

    /// Swaps two adjacent nodes: `anchor` immediately precedes `node`, and
    /// afterwards `node` immediately precedes `anchor`.
    fn move_before(&mut self, node: usize, anchor: usize) {
        let before = self.nodes[anchor].prev;
        let after = self.nodes[node].next;
        self.link(before, Some(node));
        self.link(Some(node), Some(anchor));
        self.link(Some(anchor), after);
    }

    /// Makes `b` follow `a` (either may be `None` for a list end).
    fn link(&mut self, a: Option<usize>, b: Option<usize>) {
        match a {
            Some(a) => self.nodes[a].next = b,
            None => {
                if let Some(b) = b {
                    self.head = b;
                }
            }
        }
        match b {
            Some(b) => self.nodes[b].prev = a,
            None => {
                if let Some(a) = a {
                    self.tail = a;
                }
            }
        }
    }

    /// Exchanges the tree positions of `x` and `y`.
    fn swap_tree(&mut self, x: usize, y: usize) {
        let px = self.nodes[x].parent.expect("non-root");
        let py = self.nodes[y].parent.expect("non-root");
        if px == py {
            self.nodes[px].child.swap(0, 1);
        } else {
            let sx = usize::from(self.nodes[px].child[1] == x);
            let sy = usize::from(self.nodes[py].child[1] == y);
            self.nodes[px].child[sx] = y;
            self.nodes[py].child[sy] = x;
            self.nodes[x].parent = Some(py);
            self.nodes[y].parent = Some(px);
        }
    }

    /// Raises the weight of `n` and each ancestor up to the root. Before each step the
    /// node trades places with the first node of its weight, which keeps the list
    /// sorted heaviest first and the tree a valid Huffman tree for the new weights.
    fn increment(&mut self, mut n: usize) -> Result<(), CodecError> {
        loop {
            let w = self.nodes[n].weight;
            let w1 = w.checked_add(1).ok_or(err("weight overflow"))?;
            let lead = self.leader(n);
            if lead != n {
                if self.nodes[lead].parent.is_none()
                    || self.nodes[n].parent.is_none()
                    || self.nodes[n].parent == Some(lead)
                {
                    return Err(err("swap with an ancestor"));
                }
                self.swap_list(n, lead);
                self.swap_tree(n, lead);
            }
            // `n` now heads its weight block; the block's new leader is
            // whatever follows it with the same weight. Keeping the cache
            // current saves walks; it is not needed for correctness.
            // (`n`'s predecessor weighs more than `w`, so `x` below heads
            // block `w`, and `n` heads block `w + 1` unless its
            // predecessor weighs `w + 1` too.)
            self.nodes[n].weight = w1;
            if let Some(x) = self.nodes[n].next {
                if self.nodes[x].weight == w {
                    self.leader[w as usize % LEADER_SLOTS] = x as u32;
                }
            }
            if self.nodes[n]
                .prev
                .is_none_or(|p| self.nodes[p].weight != w1)
            {
                self.leader[w1 as usize % LEADER_SLOTS] = n as u32;
            }
            match self.nodes[n].parent {
                Some(p) => n = p,
                None => return Ok(()),
            }
        }
    }

    /// Adds a leaf for byte `v` at weight 0, paired under a new branch with the old
    /// tail. Used when a stream escapes a byte that has no leaf yet.
    fn add_value(&mut self, v: u8) -> usize {
        let b = self.tail;
        let a = self.nodes.len();
        self.nodes.push(Node {
            weight: 0,
            symbol: Some(u16::from(v)),
            child: [0, 0],
            parent: None,
            prev: None,
            next: None,
        });
        self.link(Some(b), Some(a));
        self.link(Some(a), None);
        self.note_leader(a);

        let bw = self.nodes[b].weight;
        let parent = self.nodes[b].parent;
        let m = self.nodes.len();
        self.nodes.push(Node {
            weight: bw,
            symbol: None,
            child: [a, b],
            parent,
            prev: None,
            next: None,
        });
        match parent {
            Some(p) => {
                let side = usize::from(self.nodes[p].child[1] == b);
                self.nodes[p].child[side] = m;
            }
            None => self.head = m,
        }
        let before = self.nodes[b].prev;
        self.link(before, Some(m));
        self.link(Some(m), Some(b));
        self.note_leader(m);
        self.nodes[a].parent = Some(m);
        self.nodes[b].parent = Some(m);
        a
    }
}

/// List construction for `Tree::build`, and its insert rule.
#[derive(Default)]
struct Builder {
    nodes: Vec<Node>,
    head: Option<usize>,
    tail: Option<usize>,
    /// For each weight, the last node in the list with that weight.
    last_of: BTreeMap<u32, usize>,
}

impl Builder {
    fn push(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    fn insert_leaf(&mut self, sym: u16, weight: u32) {
        let i = self.push(Node {
            weight,
            symbol: Some(sym),
            child: [0, 0],
            parent: None,
            prev: None,
            next: None,
        });
        self.insert(i);
    }

    /// Places node `i` right after the last node with weight ≥ its weight,
    /// or at the head if there is none.
    fn insert(&mut self, i: usize) {
        let w = self.nodes[i].weight;
        // The last node with weight ≥ w is the last node of the smallest
        // weight ≥ w present.
        let after = self.last_of.range(w..).next().map(|(_, &n)| n);
        match after {
            Some(a) => {
                let next = self.nodes[a].next;
                self.nodes[i].prev = Some(a);
                self.nodes[i].next = next;
                self.nodes[a].next = Some(i);
                match next {
                    Some(n) => self.nodes[n].prev = Some(i),
                    None => self.tail = Some(i),
                }
            }
            None => {
                self.nodes[i].prev = None;
                self.nodes[i].next = self.head;
                match self.head {
                    Some(h) => self.nodes[h].prev = Some(i),
                    None => self.tail = Some(i),
                }
                self.head = Some(i);
            }
        }
        self.last_of.insert(w, i);
    }
}

/// Stream bits resolved by one [`Fast`] lookup.
const FAST_BITS: u32 = 10;

/// Where the walk from the root of an unmodified tree ends after reading
/// the low `bits` bits of a [`FAST_BITS`]-bit window: at a leaf, or at
/// the branch reached after all `FAST_BITS` bits.
#[derive(Clone, Copy)]
struct Fast {
    node: u16,
    bits: u8,
}

/// The tree of weight table `t`, and its [`Fast`] table: entry `j` is the
/// walk for the window `j`.
struct Template {
    tree: Tree,
    fast: Vec<Fast>,
}

impl Template {
    fn new(weights: &[u8; 256]) -> Template {
        let tree = Tree::build(weights);
        let fast = (0..1u32 << FAST_BITS)
            .map(|j| {
                let (mut n, mut bits) = (tree.root(), 0);
                while tree.nodes[n].symbol.is_none() && bits < FAST_BITS {
                    n = tree.nodes[n].child[(j >> bits & 1) as usize];
                    bits += 1;
                }
                let node = u16::try_from(n).expect("templates have < 2^16 nodes");
                Fast {
                    node,
                    bits: bits as u8,
                }
            })
            .collect();
        Template { tree, fast }
    }
}

fn templates(table: usize) -> &'static Template {
    static TEMPLATES: OnceLock<Vec<Template>> = OnceLock::new();
    &TEMPLATES.get_or_init(|| HUFFMAN_WEIGHTS.iter().map(Template::new).collect())[table]
}

#[cfg(any(test, feature = "test-support"))]
fn template(table: usize) -> &'static Tree {
    &templates(table).tree
}

/// Decodes a Huffman stream into at most `max_out` bytes.
pub(crate) fn decompress(input: &[u8], max_out: usize) -> Result<Vec<u8>, CodecError> {
    let mut r = BitReader::new(input);
    let table = r.read(8)? as usize;
    if table >= HUFFMAN_WEIGHTS.len() {
        return Err(err("invalid table type"));
    }
    let adaptive = table == 0;
    let template = templates(table);
    // Borrowed until the first change: tables 1–8 change only on escape.
    let mut tree = Cow::Borrowed(&template.tree);

    let mut out = Vec::with_capacity(max_out.min(input.len().saturating_mul(MAX_RATIO)));
    while out.len() < max_out {
        let mut n = tree.root();
        if let Cow::Borrowed(_) = tree {
            // The tree is still the template: resolve up to FAST_BITS bits
            // with one lookup. The window is zero-filled past the input,
            // so a walk that used a missing bit fails in `consume`, as
            // reading bit by bit would.
            let f = template.fast[r.peek(FAST_BITS) as usize];
            r.consume(u32::from(f.bits))?;
            n = usize::from(f.node);
        }
        while tree.nodes[n].symbol.is_none() {
            let bit = r.read(1)? as usize;
            n = tree.nodes[n].child[bit];
        }
        let sym = tree.nodes[n].symbol.expect("leaf");
        let byte = match sym {
            SYM_END => break,
            SYM_ESCAPE => {
                let v = r.read(8)? as u8;
                let t = tree.to_mut();
                n = t.add_value(v);
                t.increment(n)?;
                if !adaptive {
                    t.increment(n)?;
                }
                v
            }
            s => s as u8,
        };
        out.push(byte);
        if adaptive {
            tree.to_mut().increment(n)?;
        }
    }
    Ok(out)
}

/// Encodes `data` with weight table `table` (0..=8), the inverse of
/// [`decompress`]: the same tree is built and updated symbol by symbol
/// (build, increment, add a leaf), each byte is written as its leaf's
/// path from the root (child0 → 0, child1 → 1), a byte with no leaf as the
/// escape code plus 8 bits, and the end symbol last. Used by the
/// test-support MPQ writer and by tests.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn compress(table: u8, data: &[u8]) -> Vec<u8> {
    assert!(
        usize::from(table) < HUFFMAN_WEIGHTS.len(),
        "Huffman table {table} outside 0..=8"
    );
    let adaptive = table == 0;
    let mut tree = template(usize::from(table)).clone();
    // Symbol → leaf. Leaves never change symbol; `add_value` adds new ones.
    let mut leaf_of = [usize::MAX; 0x102];
    for (i, n) in tree.nodes.iter().enumerate() {
        if let Some(s) = n.symbol {
            leaf_of[usize::from(s)] = i;
        }
    }

    let mut w = Bits::default();
    w.put(u32::from(table), 8);
    let mut path = Vec::new();
    for &b in data {
        let n = match leaf_of[usize::from(b)] {
            usize::MAX => {
                w.path(&tree, leaf_of[usize::from(SYM_ESCAPE)], &mut path);
                w.put(u32::from(b), 8);
                let n = tree.add_value(b);
                leaf_of[usize::from(b)] = n;
                tree.increment(n).expect("valid tree");
                if !adaptive {
                    tree.increment(n).expect("valid tree");
                }
                n
            }
            n => {
                w.path(&tree, n, &mut path);
                n
            }
        };
        if adaptive {
            tree.increment(n).expect("valid tree");
        }
    }
    w.path(&tree, leaf_of[usize::from(SYM_END)], &mut path);
    w.finish()
}

/// LSB-first bit writer for [`compress`].
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    acc: u64,
    count: u32,
}

#[cfg(any(test, feature = "test-support"))]
impl Bits {
    /// Appends the low `n` (≤ 32) bits of `v`.
    fn put(&mut self, v: u32, n: u32) {
        self.acc |= (u64::from(v) & ((1u64 << n) - 1)) << self.count;
        self.count += n;
        while self.count >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.count -= 8;
        }
    }

    /// Appends the code of node `n`: the branch bits from the root down.
    /// `path` is scratch space.
    fn path(&mut self, tree: &Tree, mut n: usize, path: &mut Vec<u8>) {
        path.clear();
        while let Some(p) = tree.nodes[n].parent {
            path.push(u8::from(tree.nodes[p].child[1] == n));
            n = p;
        }
        for &bit in path.iter().rev() {
            self.put(u32::from(bit), 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpq::huffman_model::Model;

    #[test]
    fn compress_round_trip() {
        let data: Vec<u8> = b"the quick brown fox \x00\xff jumps over the lazy dog"
            .iter()
            .copied()
            .chain((0..=255u8).step_by(7))
            .collect();
        for t in 0..HUFFMAN_WEIGHTS.len() as u8 {
            let stream = compress(t, &data);
            assert_eq!(decompress(&stream, 4096).unwrap(), data, "table {t}");
        }
    }

    /// Checks the invariants every tree must keep: the list is non-increasing
    /// in weight, branches weigh the sum of their children, parent links
    /// match, and `leader` finds the first node of each weight from every
    /// node of that weight, whatever the cache holds.
    fn check(tree: &Tree) {
        let mut cur = Some(tree.head);
        let mut prev_w = u32::MAX;
        let mut seen = std::collections::HashMap::new();
        let mut count = 0;
        while let Some(i) = cur {
            let n = &tree.nodes[i];
            assert!(n.weight <= prev_w, "list order");
            prev_w = n.weight;
            seen.entry(n.weight).or_insert(i);
            if n.symbol.is_none() {
                let [c0, c1] = n.child;
                assert_eq!(n.weight, tree.nodes[c0].weight + tree.nodes[c1].weight);
                assert_eq!(tree.nodes[c0].parent, Some(i));
                assert_eq!(tree.nodes[c1].parent, Some(i));
            }
            count += 1;
            cur = n.next;
        }
        assert_eq!(count, tree.nodes.len(), "every node is in the list");
        assert_eq!(tree.nodes[tree.head].parent, None, "head is the root");
        let mut cold = tree.clone();
        cold.leader = [NO_NODE; LEADER_SLOTS];
        for (i, n) in tree.nodes.iter().enumerate() {
            assert_eq!(tree.clone().leader(i), seen[&n.weight], "leader of {i}");
            assert_eq!(cold.leader(i), seen[&n.weight], "uncached leader of {i}");
        }
    }

    /// A stale or colliding cache entry is checked, never trusted.
    #[test]
    fn leader_cache_is_only_a_hint() {
        let mut tree = template(1).clone();
        let n = tree.nodes.len();
        for c in [0u32, 1, 7, n as u32 - 1, n as u32, NO_NODE] {
            tree.leader = [c; LEADER_SLOTS];
            check(&tree);
            let mut t = tree.clone();
            let mut want = tree.clone();
            want.leader = [NO_NODE; LEADER_SLOTS];
            for i in 0..n {
                assert_eq!(t.leader(i), want.leader(i));
            }
        }
    }

    #[test]
    fn templates_are_consistent() {
        for t in 0..HUFFMAN_WEIGHTS.len() {
            check(template(t));
        }
    }

    #[test]
    fn invariants_hold_under_updates() {
        for t in [0usize, 1, 3, 6] {
            let mut tree = template(t).clone();
            for v in [7u8, 200, 7, 33, 0, 255] {
                let n = tree.add_value(v);
                tree.increment(n).unwrap();
                tree.increment(n).unwrap();
                check(&tree);
            }
            // Increment some existing leaves many times.
            let leaves: Vec<usize> = (0..tree.nodes.len())
                .filter(|&i| tree.nodes[i].symbol.is_some())
                .take(12)
                .collect();
            for round in 0..40 {
                let n = leaves[round % leaves.len()];
                tree.increment(n).unwrap();
            }
            check(&tree);
        }
    }

    /// Weights are u32 and grow with every decoded symbol; past u32::MAX
    /// (only reachable with a multi-GiB stream) `w + 1` overflowed.
    #[test]
    fn regress_weight_overflow_is_an_error() {
        let mut tree = template(0).clone();
        let root = tree.root();
        tree.nodes[root].weight = u32::MAX;
        assert!(tree.increment(root).is_err());
    }

    #[test]
    fn rejects_bad_table_type() {
        assert!(decompress(&[9, 0, 0], 16).is_err());
    }

    /// The decoder's tree in model form (node indices match: both create nodes in
    /// the same order).
    fn as_model(t: &Tree) -> Model {
        let mut l = vec![];
        let mut cur = Some(t.head);
        while let Some(i) = cur {
            l.push(i);
            cur = t.nodes[i].next;
        }
        let branch = |n: &Node| if n.symbol.is_none() { n.child } else { [0, 0] };
        Model {
            w: t.nodes.iter().map(|n| n.weight).collect(),
            sym: t.nodes.iter().map(|n| n.symbol).collect(),
            child: t.nodes.iter().map(branch).collect(),
            parent: t.nodes.iter().map(|n| n.parent).collect(),
            l,
        }
    }

    /// Deterministic byte source for the property checks.
    fn lcg_bytes(seed: u32, len: usize) -> Vec<u8> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (x >> 16) as u8
            })
            .collect()
    }

    #[test]
    fn build_matches_reference_model() {
        for (t, weights) in HUFFMAN_WEIGHTS.iter().enumerate() {
            let model = Model::build(weights);
            let tree = template(t);
            assert_eq!(as_model(tree), model, "table {t}");
            assert_eq!(tree.root(), model.root());
            // Leaves: every non-zero weight in symbol order, then 0x100, 0x101.
            let leaves: Vec<u16> = model.sym.iter().map_while(|s| *s).collect();
            let mut want: Vec<u16> = (0..256u16)
                .filter(|&s| weights[usize::from(s)] != 0)
                .collect();
            want.extend([SYM_END, SYM_ESCAPE]);
            assert_eq!(leaves, want);
        }
    }

    #[test]
    fn updates_match_reference_model() {
        for (t, weights) in HUFFMAN_WEIGHTS.iter().enumerate() {
            let mut tree = template(t).clone();
            let mut model = Model::build(weights);
            let ops = lcg_bytes(t as u32 + 1, 600);
            for (k, &op) in ops.iter().enumerate() {
                if k % 5 == 0 {
                    // AddValue, then Increment of the new leaf.
                    let a = tree.add_value(op);
                    assert_eq!(a, model.add_value(op));
                    assert_eq!(as_model(&tree), model, "table {t} add {k}");
                    tree.increment(a).unwrap();
                    model.increment(a).unwrap();
                } else {
                    // Increment an existing leaf picked by `op`.
                    let leaves: Vec<usize> = (0..model.w.len())
                        .filter(|&i| model.sym[i].is_some())
                        .collect();
                    let n = leaves[usize::from(op) % leaves.len()];
                    tree.increment(n).unwrap();
                    model.increment(n).unwrap();
                }
                assert_eq!(as_model(&tree), model, "table {t} op {k}");
            }
        }
    }

    #[test]
    fn decode_matches_reference_model() {
        let data: Vec<u8> = b"Stay awhile and listen. \x00\x01\xfe\xff"
            .iter()
            .copied()
            .chain(lcg_bytes(99, 300))
            .collect();
        for t in 0..HUFFMAN_WEIGHTS.len() as u8 {
            // Valid streams (every escape path), full and capped output.
            let stream = compress(t, &data);
            for max in [0, 1, 17, data.len(), 4096] {
                let got = decompress(&stream, max).map_err(|_| ());
                assert_eq!(got, Model::decode(&stream, max, 2), "table {t} max {max}");
            }
            assert_eq!(Model::decode(&stream, 4096, 2).unwrap(), data);
            // Arbitrary bit streams: same output or both an error.
            for seed in 0..40 {
                let mut s = vec![t];
                s.extend(lcg_bytes(seed * 9 + u32::from(t), 64));
                let got = decompress(&s, 4096).map_err(|_| ());
                assert_eq!(got, Model::decode(&s, 4096, 2), "table {t} seed {seed}");
            }
        }
    }
}
