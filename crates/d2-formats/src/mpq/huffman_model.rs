//! A plain reference model of the Huffman tree, for tests.
//!
//! The decoder in `huffman` keeps its weight-ordered node list as a linked list with a
//! leader cache, and resolves the first bits of a fresh tree with a lookup table. This
//! model does the same work the obvious way: the list `L` is a `Vec`, every lookup is a
//! linear scan, and the stream is read bit by bit. Tests hold the decoder to the model
//! node for node and byte for byte, so an optimisation can't change an output or an
//! error.

use super::huffman::{SYM_END, SYM_ESCAPE};
use super::tables::HUFFMAN_WEIGHTS;

/// The tree as parallel per-node vectors plus the weight-ordered list `L`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Model {
    /// Node weights.
    pub(super) w: Vec<u32>,
    /// `Some(symbol)` for a leaf.
    pub(super) sym: Vec<Option<u16>>,
    /// Branch children (`[0, 0]` for a leaf).
    pub(super) child: Vec<[usize; 2]>,
    pub(super) parent: Vec<Option<usize>>,
    /// The node list `L`, heaviest first; its head is the root.
    pub(super) l: Vec<usize>,
}

impl Model {
    fn node(&mut self, w: u32, sym: Option<u16>, child: [usize; 2]) -> usize {
        self.w.push(w);
        self.sym.push(sym);
        self.child.push(child);
        self.parent.push(None);
        self.w.len() - 1
    }

    fn pos(&self, n: usize) -> usize {
        self.l.iter().position(|&x| x == n).expect("in L")
    }

    /// Places `n` right after the last node with weight ≥ its own, or at the head.
    fn insert(&mut self, n: usize) {
        let w = self.w[n];
        let at = self
            .l
            .iter()
            .rposition(|&x| self.w[x] >= w)
            .map_or(0, |p| p + 1);
        self.l.insert(at, n);
    }

    /// Builds the starting tree of a weight table: a leaf for every symbol with a
    /// non-zero weight (in symbol order), then the end and escape leaves at weight 1;
    /// then, walking up from the tail, each pair of neighbours gets a parent branch.
    pub(super) fn build(weights: &[u8; 256]) -> Model {
        let mut m = Model {
            w: vec![],
            sym: vec![],
            child: vec![],
            parent: vec![],
            l: vec![],
        };
        for (s, &w) in weights.iter().enumerate() {
            if w != 0 {
                let n = m.node(u32::from(w), Some(s as u16), [0, 0]);
                m.insert(n);
            }
        }
        for s in [SYM_END, SYM_ESCAPE] {
            let n = m.node(1, Some(s), [0, 0]);
            m.insert(n);
        }
        let mut c = *m.l.last().unwrap();
        while c != m.l[0] {
            let a = c;
            let b = m.l[m.pos(a) - 1];
            let br = m.node(m.w[a] + m.w[b], None, [a, b]);
            m.parent[a] = Some(br);
            m.parent[b] = Some(br);
            m.insert(br);
            let pb = m.pos(b);
            if pb == 0 {
                break;
            }
            c = m.l[pb - 1];
        }
        m
    }

    pub(super) fn root(&self) -> usize {
        self.l[0]
    }

    /// Adds one to the weight of `n` and of every ancestor up to the root. Before
    /// each step the node trades places (in `L` and in the tree) with the first node
    /// of its weight, so `L` stays sorted.
    pub(super) fn increment(&mut self, mut n: usize) -> Result<(), ()> {
        loop {
            let w = self.w[n];
            let lead = *self.l.iter().find(|&&x| self.w[x] == w).unwrap();
            if lead != n {
                // (The implementation rejects a swap with the root or
                // with n's own parent; valid streams never reach it.)
                let (pn, pl) = (self.parent[n].ok_or(())?, self.parent[lead].ok_or(())?);
                if pn == lead {
                    return Err(());
                }
                let (a, b) = (self.pos(n), self.pos(lead));
                self.l.swap(a, b);
                if pn == pl {
                    self.child[pn].swap(0, 1);
                } else {
                    let sn = usize::from(self.child[pn][1] == n);
                    let sl = usize::from(self.child[pl][1] == lead);
                    self.child[pn][sn] = lead;
                    self.child[pl][sl] = n;
                    self.parent[n] = Some(pl);
                    self.parent[lead] = Some(pn);
                }
            }
            self.w[n] = w + 1;
            match self.parent[n] {
                Some(p) => n = p,
                None => return Ok(()),
            }
        }
    }

    /// Adds a leaf for byte `v` at weight 0: it becomes the new tail, paired under a
    /// new branch with the old tail.
    pub(super) fn add_value(&mut self, v: u8) -> usize {
        let a = self.node(0, Some(u16::from(v)), [0, 0]);
        self.insert(a);
        assert_eq!(*self.l.last().unwrap(), a, "the new leaf is the tail");
        let b = self.l[self.pos(a) - 1];
        let m = self.node(self.w[b], None, [a, b]);
        let pb = self.parent[b];
        if let Some(p) = pb {
            let side = usize::from(self.child[p][1] == b);
            self.child[p][side] = m;
        }
        self.parent[m] = pb;
        let at = self.pos(b);
        self.l.insert(at, m);
        self.parent[a] = Some(m);
        self.parent[b] = Some(m);
        a
    }

    /// Decodes a stream: the first byte picks the weight table, then symbols are read
    /// root to leaf until the end symbol or `max_out` bytes. An escape is followed by a
    /// literal byte that gets a new leaf, incremented once. Table 0 is adaptive: every
    /// decoded symbol (a new leaf too) is then incremented. Tables 1–8 are not, but a new
    /// leaf gets `escape_increments` in total (the decoder uses 2; tests also try 1 to
    /// show that the second one changes the output).
    pub(crate) fn decode(
        input: &[u8],
        max_out: usize,
        escape_increments: u32,
    ) -> Result<Vec<u8>, ()> {
        let mut bit = 0usize;
        let mut read = |n: u32| -> Result<u32, ()> {
            let mut v = 0;
            for k in 0..n {
                let byte = *input.get(bit / 8).ok_or(())?;
                v |= u32::from(byte >> (bit % 8) & 1) << k;
                bit += 1;
            }
            Ok(v)
        };
        let t = read(8)? as usize;
        if t > 8 {
            return Err(());
        }
        let adaptive = t == 0;
        let mut m = Model::build(&HUFFMAN_WEIGHTS[t]);
        let mut out = Vec::new();
        while out.len() < max_out {
            let mut n = m.root();
            while m.sym[n].is_none() {
                n = m.child[n][read(1)? as usize];
            }
            let sym = match m.sym[n].unwrap() {
                SYM_END => break,
                SYM_ESCAPE => {
                    let v = read(8)? as u8;
                    n = m.add_value(v);
                    m.increment(n)?;
                    if !adaptive && escape_increments == 2 {
                        m.increment(n)?;
                    }
                    v
                }
                s => s as u8,
            };
            out.push(sym);
            if adaptive {
                m.increment(n)?;
            }
        }
        Ok(out)
    }
}
