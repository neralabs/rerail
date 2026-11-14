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
//! - **The kind-set axes** (`Mode::Axis`, `core.rr`'s `nearest`, `inside` and `has`; decisions 0036 and 0057): `rr_nearest_<set>(n, a)`, `a` the nearest strict ancestor of `n` of a kind in
//!   the set; `rr_inside_<set>(n, a)`, every such ancestor; `rr_has_<set>(n, a)`, `n` of a kind in the set and `a`
//!   each of its strict ancestors. The same climbs and subtree intervals, filtered by the set.
//!
//! - **The batch's solve** (`Nearest` and `Inside`, decision 0105): where the planner expects more `n` lookups than
//!   one solve of every node costs, the `n` side is made for the whole batch at once by the transducer's file-local
//!   FIX (`typing::file_local::nearest`: pointer jumping on Metal, one pass in preorder on the CPU). Its rows are ids
//!   `0..`; the rows made per looked-up `n` are then ids from `LOOKED_UP` on. A scan of these axes solves.
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
pub enum Mode {
    /// Strictly below in the tree.
    Tree,
    /// Span inside span, equal spans included.
    Span,
    /// Strictly below an ancestor of a kind in a set (`Within::axis`).
    Axis(Axis),
}

/// A kind-set axis: which ancestors of `n` are its rows' `a`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// The nearest one in the set.
    Nearest,
    /// Every one in the set.
    Inside,
    /// Every one, where `n` is in the set.
    Has,
}

/// Rows live in chunks of doubling size: chunk `k` holds `BASE << k` rows.
const BASE: usize = 1024;
const CHUNKS: usize = 22;
/// The first id of the rows made from `outer`.
const OUTER: usize = 1 << 31;
/// The first id of the rows made per looked-up `n` of an axis the batch's solve answers (`Within::solvable`).
const LOOKED_UP: usize = 1 << 30;
/// What one `n` made on lookup costs, in nanoseconds: the climb, its rows stored and the run kept under the lock.
const LOOKUP_NS: f64 = 150.0;
/// What the batch's solve costs per node, in nanoseconds: the parents and memberships copied, the FIX's pass,
/// and the rows written.
const SOLVE_NS: f64 = 12.0;

pub struct Within {
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
    /// The rows' count and per column its distinct values and the sum of its postings' squares, from the tree.
    shape: OnceLock<Shape>,
    len: AtomicUsize,
    /// An axis's set: per dialect tag, whether each kind code is in it.
    members: FxHashMap<u8, Box<[bool]>>,
    /// Where the batch's solve runs.
    binding: crate::code_rule_dsl::facts::nearest::Binding,
    /// The `n` lookups the planner expects, summed over its plans (`expect_lookups`).
    expected: Mutex<f64>,
    /// The batch's solve chosen: the next `n` lookup reads it.
    chosen: std::sync::atomic::AtomicBool,
    /// The `n` side of every node, made once (`solved`).
    solved: OnceLock<Solved>,
}

/// An axis's `n` side for every node of the batch: per node row its run in `pairs` (`start[row]..start[row + 1]`),
/// each pair inner node row << 32 | outer node row.
struct Solved {
    start: Vec<u32>,
    pairs: Vec<u64>,
}

/// Statistics from the tree (`Within::shape`): rows, and per column (`n`, `a`) distinct values and the sum of
/// squared postings.
#[derive(Clone, Copy)]
struct Shape {
    rows: i64,
    ndv: [i64; 2],
    sq: [i64; 2],
}

fn slot(row: usize) -> (usize, usize) {
    let x = row / BASE + 1;
    let k = (usize::BITS - 1 - x.leading_zeros()) as usize;
    (k, row - BASE * ((1 << k) - 1))
}

impl Within {
    pub fn new(batch: Arc<Batch>, mode: Mode) -> Self {
        Within {
            batch,
            mode,
            chunks: std::array::from_fn(|_| std::array::from_fn(|_| OnceLock::new())),
            made: Mutex::new(([0, 0], [FxHashMap::default(), FxHashMap::default()])),
            all: OnceLock::new(),
            zero: OnceLock::new(),
            shape: OnceLock::new(),
            len: AtomicUsize::new(0),
            members: FxHashMap::default(),
            binding: crate::code_rule_dsl::facts::nearest::Binding::Cpu,
            expected: Mutex::new(0.0),
            chosen: std::sync::atomic::AtomicBool::new(false),
            solved: OnceLock::new(),
        }
    }

    /// The axis `axis` over the set of kind atoms `kinds`, its batch's solve run on `binding`. Nothing is made until
    /// a join reads it.
    pub fn axis(batch: Arc<Batch>, axis: Axis, kinds: &[i64], binding: crate::code_rule_dsl::facts::nearest::Binding) -> Self {
        use crate::code_rule_dsl::facts::core::{KIND_STRIDE, kind_atom};
        let kinds: rustc_hash::FxHashSet<i64> = kinds.iter().copied().collect();
        let members = batch
            .dialects
            .iter()
            .map(|(&tag, d)| {
                let table = (0..KIND_STRIDE as usize)
                    .map(|code| kinds.contains(&i64::from(kind_atom(tag, d.canonical.get(code).copied().unwrap_or(code as u16)))))
                    .collect();
                (tag, table)
            })
            .collect();
        Within { members, binding, ..Within::new(batch, Mode::Axis(axis)) }
    }

    /// Whether the batch's solve answers the `n` side: an axis of ancestors in the set (`has` reads every ancestor).
    fn solvable(&self) -> bool {
        matches!(self.mode, Mode::Axis(Axis::Nearest | Axis::Inside))
    }

    /// The first id of the rows made per looked-up `n`.
    fn looked_up(&self) -> usize {
        if self.solvable() { LOOKED_UP } else { 0 }
    }

    /// The batch's solve: every node's nearest strict ancestor in the set by the transducer's FIX, then per node its
    /// rows (that ancestor, or for `inside` the chain of them).
    fn solved(&self) -> &Solved {
        self.solved.get_or_init(|| {
            use crate::code_rule_dsl::logic::profile;
            use crate::code_rule_dsl::facts::nearest::{self as file_local, Binding, Trees};
            let _span = profile::span(|| "kind-set axis solved for the batch".into());
            let n = self.batch.nodes.last().copied().unwrap_or(0);
            let (mut parent, mut kind) = (Vec::with_capacity(n), Vec::with_capacity(n));
            for (doc, p) in self.batch.docs.iter().enumerate() {
                let base = self.batch.nodes[doc] as u32;
                for l in 0..p.len() {
                    let up = p.parent(l as usize);
                    parent.push(if up == NONE { file_local::NONE } else { base + up });
                    kind.push(i64::from(self.member(doc, l)));
                }
            }
            profile::work(profile::Unit::Node, n as u64);
            // Membership is the kind: the set is `{1}`.
            let trees = Trees { parent: &parent, kind: &kind };
            let found = file_local::nearest(&trees, &[vec![1]], self.binding)
                .or_else(|_| file_local::nearest(&trees, &[vec![1]], Binding::Cpu))
                .expect("the CPU's nearest pass")
                .pop()
                .unwrap_or_default();
            let inside = self.mode == Mode::Axis(Axis::Inside);
            let (mut start, mut pairs) = (Vec::with_capacity(n + 1), Vec::with_capacity(n));
            for v in 0..found.len() {
                start.push(pairs.len() as u32);
                let mut a = found[v];
                while a != file_local::NONE {
                    pairs.push((v as u64) << 32 | u64::from(a));
                    a = if inside { found[a as usize] } else { file_local::NONE };
                }
            }
            start.push(pairs.len() as u32);
            Solved { start, pairs }
        })
    }

    /// The planner expects `keys` more lookups of `n`: the batch's solve is chosen once they cost more than it.
    pub(super) fn expect(&self, keys: f64) {
        if !self.solvable() || self.chosen.load(Ordering::Relaxed) {
            return;
        }
        let mut expected = self.expected.lock();
        *expected += keys.max(0.0);
        let nodes = self.batch.nodes.last().copied().unwrap_or(0) as f64;
        if *expected * LOOKUP_NS > nodes * SOLVE_NS {
            self.chosen.store(true, Ordering::Relaxed);
            crate::code_rule_dsl::logic::profile::decision("kind-set axes (decision 0105)", || {
                format!("solved for the batch: the planner expects at least {:.0} lookups over {nodes:.0} nodes", *expected)
            });
        }
    }

    /// Whether `n` lookups read the batch's solve.
    fn reads_solved(&self) -> bool {
        self.solvable() && (self.chosen.load(Ordering::Relaxed) || self.solved.get().is_some())
    }

    /// Whether node `local` of document `doc` is of a kind in the axis's set.
    fn member(&self, doc: usize, local: u32) -> bool {
        let p = &self.batch.docs[doc];
        let code = crate::code_rule_dsl::facts::core::kind_code(p.facts.kind[p.core[local as usize] as usize]);
        self.members.get(&p.tag).is_some_and(|t| t.get(usize::from(code)).is_some_and(|&m| m))
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
        if let Mode::Axis(axis) = self.mode {
            let mut out = Vec::new();
            if axis == Axis::Has && !self.member(doc, local) {
                return out;
            }
            let mut up = p.parent(local as usize);
            while up != NONE {
                if axis == Axis::Has || self.member(doc, up) {
                    out.push(up);
                    if axis == Axis::Nearest {
                        break;
                    }
                }
                up = p.parent(up as usize);
            }
            return out;
        }
        if self.mode == Mode::Tree {
            let mut out = Vec::new();
            let mut up = p.parent(local as usize);
            while up != NONE {
                out.push(up);
                up = p.parent(up as usize);
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
        let mut up = p.parent(local as usize);
        while up != NONE {
            if holds(up) {
                out.push(up);
            }
            up = p.parent(up as usize);
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
        let sizes = p.sizes();
        if let Mode::Axis(axis) = self.mode {
            let end = local + sizes[local as usize];
            return match axis {
                _ if axis != Axis::Has && !self.member(doc, local) => Vec::new(),
                Axis::Inside => (local + 1..=end).collect(),
                Axis::Has => (local + 1..=end).filter(|&n| self.member(doc, n)).collect(),
                Axis::Nearest => {
                    // The subtree, less what lies below a nearer member: its nearest is that member or lower.
                    let (mut out, mut n) = (Vec::new(), local + 1);
                    while n <= end {
                        out.push(n);
                        n += if self.member(doc, n) { sizes[n as usize] + 1 } else { 1 };
                    }
                    out
                }
            };
        }
        if self.mode == Mode::Tree {
            return (local + 1..=local + sizes[local as usize]).collect();
        }
        let (s, e) = self.span(doc, local);
        let end = local + sizes[local as usize];
        let mut out: Vec<u32> = (local..=end).collect();
        let mut up = p.parent(local as usize);
        while up != NONE && self.span(doc, up) == (s, e) {
            out.push(up);
            up = p.parent(up as usize);
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
        assert!(start + others.len() <= if side == 0 && self.solvable() { OUTER - LOOKED_UP } else { OUTER }, "within: more rows than ids");
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
        let run = ((start + if side == 0 { self.looked_up() } else { OUTER }) as u32, others.len() as u32);
        made.1[side].insert(row, run);
        run
    }

    fn names(&self) -> [&'static str; 2] {
        match self.mode {
            Mode::Tree | Mode::Axis(_) => ["n", "a"],
            Mode::Span => ["inner", "outer"],
        }
    }

    fn pair(&self, row: usize) -> u64 {
        if self.solvable() && row < LOOKED_UP {
            return self.solved().pairs[row];
        }
        let side = usize::from(row >= OUTER);
        let (k, at) = slot(row - if side == 0 { self.looked_up() } else { OUTER });
        self.chunks[side][k].get().expect("a row made")[at].load(Ordering::Relaxed)
    }

    fn shape(&self) -> Shape {
        *self.shape.get_or_init(|| {
            if let Mode::Axis(axis) = self.mode {
                return self.axis_shape(axis);
            }
            let (mut rows, mut nodes, mut ancestors, mut sq_inner) = (0i64, 0i64, 0i64, 0i64);
            for p in &self.batch.docs {
                let (depths, sizes) = (p.depths(), p.sizes());
                for l in 0..p.len() as usize {
                    // Span mode counts the node itself on both sides.
                    let own = i64::from(self.mode == Mode::Span);
                    let (d, s) = (i64::from(depths[l]) + own, i64::from(sizes[l]) + own);
                    rows += d;
                    nodes += 1;
                    ancestors += i64::from(s > 0);
                    sq_inner += d * d;
                }
            }
            // The ancestor side is priced by its mean subtree size, as the planner prices `nearest`'s descent: the
            // size-weighted mean (the sum of squared subtree sizes over rows) is the subtree of a pair's ancestor,
            // which a document's root and its largest classes make about a thousand nodes, while the ancestors a
            // rule binds (its functions, blocks, statements) hold tens. Priced so, a rule from a function's nodes
            // scanned every containment pair rather than look up each function's subtree.
            let ancestors = ancestors.max(1);
            let sq_outer = rows.saturating_mul(rows) / ancestors;
            Shape { rows, ndv: [nodes, ancestors], sq: [sq_inner, sq_outer] }
        })
    }

    /// An axis's statistics, from one pass over each document's nodes in preorder: its members, the nodes below
    /// one (`covered`), the members' subtree sizes and depths. Exact rows; postings of the side that holds several
    /// rows per value are spread evenly where the tree does not give them.
    fn axis_shape(&self, axis: Axis) -> Shape {
        let (mut members, mut covered, mut below, mut below_sq, mut above, mut above_sq, mut nodes) = (0i64, 0i64, 0i64, 0i64, 0i64, 0i64, 0i64);
        for (doc, p) in self.batch.docs.iter().enumerate() {
            // The first member demands these columns; documents without members allocate neither.
            let mut columns = None;
            // The end of the outermost member's subtree the walk is in.
            let mut until: Option<u32> = None;
            for l in 0..p.len() {
                nodes += 1;
                if until.is_some_and(|u| l <= u) {
                    covered += 1;
                }
                if !self.member(doc, l) {
                    continue;
                }
                let (sizes, depths) = *columns.get_or_insert_with(|| (p.sizes(), p.depths()));
                if until.is_none_or(|u| l > u) {
                    until = Some(l + sizes[l as usize]);
                }
                let (s, d) = (i64::from(sizes[l as usize]), i64::from(depths[l as usize]));
                members += 1;
                below += s;
                below_sq += s * s;
                above += d;
                above_sq += d * d;
            }
        }
        let spread = |rows: i64, ndv: i64| rows * rows / ndv.max(1);
        match axis {
            // A member that is a leaf holds no row: at most one value per row.
            Axis::Nearest => Shape { rows: covered, ndv: [covered, members.min(covered)], sq: [covered, spread(covered, members.min(covered))] },
            Axis::Inside => Shape { rows: below, ndv: [covered, members.min(below)], sq: [spread(below, covered), below_sq] },
            Axis::Has => {
                let ancestors = above.min(nodes);
                Shape { rows: above, ndv: [members, ancestors], sq: [above_sq, spread(above, ancestors)] }
            }
        }
    }
}

impl Source for Within {
    fn columns(&self) -> Vec<String> {
        self.names().iter().map(|&c| c.to_owned()).collect()
    }
    /// Every pair once: a scan makes each node's containers, ids `0..len`.
    fn len(&self) -> usize {
        if self.solvable() {
            return self.solved().pairs.len();
        }
        *self.all.get_or_init(|| {
            for row in 0..self.batch.nodes.last().copied().unwrap_or(0) as u32 {
                self.rows(0, row);
            }
            self.len.load(Ordering::Acquire)
        })
    }
    fn estimated_len(&self) -> usize {
        self.shape().rows as usize
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let shape = self.shape();
        let per = (shape.rows / shape.ndv[column.min(1)].max(1)).max(1) as usize;
        if batch { per * 8 } else { per }
    }
    fn indexed(&self, column: usize) -> bool {
        column < 2
    }
    fn made_on_lookup(&self) -> bool {
        true
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        if column == 0 {
            self.expect(keys);
        }
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut out = Vec::new();
        for &v in values {
            let Some((doc, local)) = self.batch.node(v) else { continue };
            let row = self.batch.nodes[doc] + local as usize;
            if column == 0 && self.reads_solved() {
                let start = &self.solved().start;
                out.extend(start[row] as usize..start[row + 1] as usize);
                continue;
            }
            let (start, n) = self.rows(column.min(1), row as u32);
            out.extend(start as usize..(start + n) as usize);
        }
        Some(out)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let pair = self.pair(row);
        self.batch.id(if column == 0 { (pair >> 32) as usize } else { (pair & 0xffff_ffff) as usize })
    }
    /// From the tree (`shape`): for containment, as many rows as nodes have containers (their depth + 1), per
    /// inner node its depth + 1, per outer node its subtree. Values never cross documents.
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        let Shape { rows, ndv, sq } = self.shape();
        let [inner, outer] = self.names();
        Some(vec![(inner.to_owned(), rows, ndv[0], sq[0], sq[0]), (outer.to_owned(), rows, ndv[1], sq[1], sq[1])])
    }
}
