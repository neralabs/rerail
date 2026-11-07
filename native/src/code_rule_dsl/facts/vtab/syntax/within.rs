//! Containment read off the tree, made per looked-up node (decision 0050), in two modes:
//!
//! - **`within(n, a)`** (`Mode::Tree`, the language's built-in, `core.rr`): `n` is strictly below `a`. The
//!   containers of `n` are its ancestors; the nodes `a` holds are its subtree, a preorder interval.
//! - **`span_within(inner, outer)`** (`Mode::Span`, what `simplify::span_containment` makes of two span
//!   comparisons): `outer`'s span holds `inner`'s, in one document. Exactly the pairs of `node(d, inner, _, s, e), node(d, outer, _, s2, e2),
//! s2 <= s, e <= e2`, read off the tree instead of compared for every pair of a file's nodes:
//!
//! - **The containers of a node** (`inner` bound): the node, its ancestors, and its descendants with the
//!   same span. Spans of a syntax tree nest or are disjoint (siblings never overlap), so no other node of
//!   positive width holds it. A node of zero width at `p` is held by every node whose span touches `p`,
//!   siblings included: those are found by reading the document's nodes.
//! - **The nodes it holds** (`outer` bound): its subtree, its ancestors with the same span, and the
//!   document's nodes of zero width inside its span.
//!
//! Rows are made per node a join looks up (nothing for a node never bound), kept for the batch, and
//! never moved: a row's id is stable, and reading it takes no lock. The two sides keep their rows apart:
//! rows made from `inner` are ids `0..`, rows made from `outer` ids from `OUTER` on. A scan makes every
//! node's containers and reads ids `0..len`, so it sees each pair once. The planner's statistics come from the tree (depths and subtree sizes), not the rows.
use super::{Batch, NONE};
use crate::code_rule_dsl::facts::vtab::Source;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

/// Which containment the rows are.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// Strictly below in the tree.
    Tree,
    /// Span inside span, equal spans included.
    Span,
}

/// Rows live in chunks of doubling size: chunk `k` holds `BASE << k` rows.
const BASE: usize = 1024;
const CHUNKS: usize = 22;
/// The first id of the rows made from `outer`.
const OUTER: usize = 1 << 31;

pub(super) struct Within {
    batch: Arc<Batch>,
    mode: Mode,
    /// Per side, each row: inner node row << 32 | outer node row (rows of the batch's `node`).
    chunks: [[OnceLock<Box<[AtomicU64]>>; CHUNKS]; 2],
    /// Per side, the rows made and per node row the run of its rows as inner (`0`) or outer (`1`).
    made: Mutex<([usize; 2], [FxHashMap<u32, (u32, u32)>; 2])>,
    /// Every node's containers made (a scan): the rows they span.
    all: OnceLock<usize>,
    /// Per document, its nodes of zero width (locals).
    zero: OnceLock<Vec<Vec<u32>>>,
    /// Rows (the sum of depth + 1), the sum of squared containers and of squared subtree sizes.
    shape: OnceLock<(i64, i64, i64, i64)>,
    len: AtomicUsize,
}

fn slot(row: usize) -> (usize, usize) {
    let x = row / BASE + 1;
    let k = (usize::BITS - 1 - x.leading_zeros()) as usize;
    (k, row - BASE * ((1 << k) - 1))
}

impl Within {
    pub(super) fn new(batch: Arc<Batch>, mode: Mode) -> Self {
        Within {
            batch,
            mode,
            chunks: std::array::from_fn(|_| std::array::from_fn(|_| OnceLock::new())),
            made: Mutex::new(([0, 0], [FxHashMap::default(), FxHashMap::default()])),
            all: OnceLock::new(),
            zero: OnceLock::new(),
            shape: OnceLock::new(),
            len: AtomicUsize::new(0),
        }
    }

    /// The document and local node of node row `row`.
    fn at(&self, row: u32) -> (usize, u32) {
        let doc = self.batch.nodes.partition_point(|&o| o <= row as usize) - 1;
        (doc, row - self.batch.nodes[doc] as u32)
    }

    fn span(&self, doc: usize, local: u32) -> (u32, u32) {
        let p = &self.batch.docs[doc];
        let id = p.core[local as usize] as usize;
        (p.facts.start[id], p.facts.end[id])
    }

    fn zero(&self) -> &Vec<Vec<u32>> {
        self.zero.get_or_init(|| {
            (0..self.batch.docs.len())
                .map(|doc| (0..self.batch.docs[doc].len()).filter(|&l| {
                    let (s, e) = self.span(doc, l);
                    s == e
                }).collect())
                .collect()
        })
    }

    /// The nodes (locals) that hold `local`.
    fn containers(&self, doc: usize, local: u32) -> Vec<u32> {
        let p = &self.batch.docs[doc];
        if self.mode == Mode::Tree {
            let mut out = Vec::new();
            let mut up = p.parent[local as usize];
            while up != NONE {
                out.push(up);
                up = p.parent[up as usize];
            }
            return out;
        }
        let (s, e) = self.span(doc, local);
        let holds = |b: u32| {
            let (bs, be) = self.span(doc, b);
            bs <= s && e <= be
        };
        if s == e {
            return (0..p.len()).filter(|&b| holds(b)).collect();
        }
        let mut out = vec![local];
        let mut up = p.parent[local as usize];
        while up != NONE {
            if holds(up) {
                out.push(up);
            }
            up = p.parent[up as usize];
        }
        // Descendants with the same span: down through children of that span.
        let mut stack = vec![local];
        while let Some(n) = stack.pop() {
            for c in p.children(n) {
                if self.span(doc, c) == (s, e) {
                    out.push(c);
                    stack.push(c);
                }
            }
        }
        out
    }

    /// The nodes (locals) that `local` holds.
    fn contained(&self, doc: usize, local: u32) -> Vec<u32> {
        let p = &self.batch.docs[doc];
        if self.mode == Mode::Tree {
            return (local + 1..=local + p.size[local as usize]).collect();
        }
        let (s, e) = self.span(doc, local);
        let end = local + p.size[local as usize];
        let mut out: Vec<u32> = (local..=end).collect();
        let mut up = p.parent[local as usize];
        while up != NONE && self.span(doc, up) == (s, e) {
            out.push(up);
            up = p.parent[up as usize];
        }
        let (subtree, held) = ((end - local + 1) as usize, out.len());
        for &z in &self.zero()[doc] {
            let (zs, _) = self.span(doc, z);
            if s <= zs && zs <= e && !(local..=end).contains(&z) && !out[subtree..held].contains(&z) {
                out.push(z);
            }
        }
        out
    }

    /// The rows of node row `row` as inner (`side` 0) or outer (1), made on the first lookup.
    fn rows(&self, side: usize, row: u32) -> (u32, u32) {
        if let Some(&run) = self.made.lock().1[side].get(&row) {
            return run;
        }
        let (doc, local) = self.at(row);
        let base = self.batch.nodes[doc] as u32;
        let others = if side == 0 { self.containers(doc, local) } else { self.contained(doc, local) };
        let mut made = self.made.lock();
        if let Some(&run) = made.1[side].get(&row) {
            return run;
        }
        let start = made.0[side];
        assert!(start + others.len() <= OUTER, "within: more rows than ids");
        for (i, other) in others.iter().enumerate() {
            let (inner, outer) = if side == 0 { (row, base + other) } else { (base + other, row) };
            let (k, at) = slot(start + i);
            let chunk = self.chunks[side][k].get_or_init(|| (0..BASE << k).map(|_| AtomicU64::new(0)).collect());
            chunk[at].store(u64::from(inner) << 32 | u64::from(outer), Ordering::Relaxed);
        }
        made.0[side] += others.len();
        if side == 0 {
            self.len.store(made.0[0], Ordering::Release);
        }
        let run = ((start + side * OUTER) as u32, others.len() as u32);
        made.1[side].insert(row, run);
        run
    }

    fn names(&self) -> [&'static str; 2] {
        match self.mode {
            Mode::Tree => ["n", "a"],
            Mode::Span => ["inner", "outer"],
        }
    }

    fn pair(&self, row: usize) -> u64 {
        let side = usize::from(row >= OUTER);
        let (k, at) = slot(row - side * OUTER);
        self.chunks[side][k].get().expect("a row made")[at].load(Ordering::Relaxed)
    }

    fn shape(&self) -> (i64, i64, i64, i64) {
        *self.shape.get_or_init(|| {
            let (mut rows, mut nodes, mut sq_inner, mut sq_outer) = (0i64, 0i64, 0i64, 0i64);
            for p in &self.batch.docs {
                for l in 0..p.len() as usize {
                    // Span mode counts the node itself on both sides.
                    let own = i64::from(self.mode == Mode::Span);
                    let (d, s) = (i64::from(p.depth[l]) + own, i64::from(p.size[l]) + own);
                    rows += d;
                    nodes += 1;
                    sq_inner += d * d;
                    sq_outer += s * s;
                }
            }
            (rows, nodes, sq_inner, sq_outer)
        })
    }
}

impl Source for Within {
    fn columns(&self) -> Vec<String> {
        self.names().iter().map(|&c| c.to_owned()).collect()
    }
    /// Every pair once: a scan makes each node's containers, ids `0..len`.
    fn len(&self) -> usize {
        *self.all.get_or_init(|| {
            for row in 0..self.batch.nodes.last().copied().unwrap_or(0) as u32 {
                self.rows(0, row);
            }
            self.len.load(Ordering::Acquire)
        })
    }
    fn estimated_len(&self) -> usize {
        self.shape().0 as usize
    }
    fn estimated_matches(&self, _column: usize, batch: bool) -> usize {
        let (rows, nodes, _, _) = self.shape();
        let per = (rows / nodes.max(1)).max(1) as usize;
        if batch { per * 8 } else { per }
    }
    fn indexed(&self, column: usize) -> bool {
        column < 2
    }
    fn made_on_lookup(&self) -> bool {
        true
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut out = Vec::new();
        for &v in values {
            let Some((doc, local)) = self.batch.node(v) else { continue };
            let (start, n) = self.rows(column.min(1), (self.batch.nodes[doc] + local as usize) as u32);
            out.extend(start as usize..(start + n) as usize);
        }
        Some(out)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let pair = self.pair(row);
        self.batch.id(if column == 0 { (pair >> 32) as usize } else { (pair & 0xffff_ffff) as usize })
    }
    /// From the tree: as many rows as nodes have containers (their depth + 1); per inner node its
    /// depth + 1, per outer node its subtree. Values never cross documents.
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        let (rows, nodes, sq_inner, sq_outer) = self.shape();
        let [inner, outer] = self.names();
        Some(vec![(inner.to_owned(), rows, nodes, sq_inner, sq_inner), (outer.to_owned(), rows, nodes, sq_outer, sq_outer)])
    }
}
