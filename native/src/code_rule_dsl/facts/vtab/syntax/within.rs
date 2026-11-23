//! Containment read off the tree, made per looked-up node (decision 0050), in two modes:
//!
//! - **`within(n, a)`** (`Mode::Tree`, the language's built-in, `core.rerail`): `n` is strictly below `a`. The
//!   containers of `n` are its ancestors; the nodes `a` holds are its subtree, a preorder interval.
//! - **`span_within(inner, outer)`** (`Mode::Span`, what `simplify::SpanContainment` makes of two span
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
//! - **The kind-set axes** (`Mode::Axis`, `core.rerail`'s `nearest`, `inside` and `has`; decisions 0036 and 0057): `rr_nearest_<set>(n, a)`, `a` the nearest strict ancestor of `n` of a kind in
//!   the set; `rr_inside_<set>(n, a)`, every such ancestor; `rr_has_<set>(n, a)`, `n` of a kind in the set and `a`
//!   each of its strict ancestors. The same climbs and subtree intervals, filtered by the set.
//!
//! - **The batch's solve** (`Nearest` and `Inside`, decision 0143): where the planner expects more `n` lookups than
//!   one solve of every node costs, the `n` side is made for the whole batch at once by the transducer's file-local
//!   FIX (`typing::file_local::nearest`: pointer jumping on Metal, one pass in preorder on the CPU). Its rows are ids
//!   `0..`; the rows made per looked-up `n` are then ids from `LOOKED_UP` on. A scan of these axes solves.
//!
//! - **On the tape** (decision 0293, step 3): where a document's facts hold the tape they were read off
//!   (`Facts::tape`), the kind-set axes and the tree's containment read it in place, in the run's format
//!   (`RERAIL_TAPE`): membership is the code's (the set as the tape's code numbers, made once a document), climbs are
//!   the tape's parents, subtrees its intervals, and the batch's CPU solve one pass of the set's matches (`nearest_all`:
//!   no parent read). The projection's nodes are the tape's, kept where `Projected::local` has them. Spans, the
//!   statistics' sizes and depths, and the solve on Metal read the facts until steps 4 and 5.
//!
//! Rows are made per node a join looks up (nothing for a node never bound), kept for the batch, and
//! never moved: a row's id is stable, and reading it takes no lock. The two sides keep their rows apart:
//! rows made from `inner` are ids `0..`, rows made from `outer` ids from `OUTER` on. A scan makes every
//! node's containers and reads ids `0..len`, so it sees each pair once. The planner's statistics come from the tree (depths and subtree sizes), not the rows.
use super::{Batch, NONE, Projected, View};
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::tape::formats::{DocTape, Format};
use rerail_tape_format::tape::{CodeSet, Tape, nearest_all};
use rerail_tape_format::with_tape;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use crate::code_rule_dsl::facts::nearest::Binding;

/// Which containment the rows are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Strictly below in the tree.
    Tree,
    /// Span inside span, equal spans included.
    Span,
    /// Strictly below an ancestor of a kind in a set (`Within::axis`).
    Axis(Axis),
}

/// A kind-set axis: which ancestors of `n` are its rows' `a`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// The set's kind atoms, ascending: what the device tests a node's kind against (`has_walk`).
    kinds: Arc<[i64]>,
    /// Where the batch's solve runs.
    binding: crate::code_rule_dsl::facts::nearest::Binding,
    /// The `n` lookups the planner expects, summed over its plans (`expect_lookups`).
    expected: Mutex<f64>,
    /// The batch's solve chosen: the next `n` lookup reads it.
    chosen: std::sync::atomic::AtomicBool,
    /// The `n` side of every node, made once (`solved`): its own, or another axis's over the same set (`solves`).
    solved: OnceLock<Arc<Solved>>,
    /// The solves of the batch's axes over its set (`Solves`), where it shares them.
    solves: Option<Arc<Solves>>,
    /// The format a test reads the documents' tapes in (`reading`); else each is read as held.
    format: Option<Format>,
    /// Per document, the axis's set as its tape's code numbers (all of them for the tree's containment).
    sets: Box<[OnceLock<Arc<CodeSet>>]>,
    /// The sets made, by language and table: format 16's documents share their language's table (decision 0336 §7),
    /// so the set is made once for all of them, not once per document over the whole table.
    shared: Mutex<FxHashMap<(u16, usize), Arc<CodeSet>>>,
}

/// An axis's `n` side for every node of the batch, a derived relation written once where the solve computes it
/// (decision 0353 step 5): its rows, the ids of `n` and `a`, and per node row its run of them (`start[row]..start[row +
/// 1]`). Made on the host, or on the device from the batch's chunk block (decision 0289 §2, its starts in the chunk);
/// both the host and the device read the relation where it lies.
struct Solved {
    start: Vec<u32>,
    rows: crate::code_rule_dsl::facts::rows::Part,
    /// The starts' offset in the chunk, when the device made them.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    start_in_chunk: Option<usize>,
}

/// The solves of a batch's axes over one set of kinds, each mode's made once for all of them (decision 0494 step 7):
/// an `inside` whose set nests nowhere is its `nearest`, the same rows.
#[derive(Default)]
pub struct Solves {
    nearest: OnceLock<Arc<Solved>>,
    inside: OnceLock<Arc<Solved>>,
}

/// Statistics from the tree (`Within::shape`): rows, and per column (`n`, `a`) distinct values and the sum of
/// squared postings.
#[derive(Clone, Copy)]
struct Shape {
    rows: i64,
    ndv: [i64; 2],
    sq: [i64; 2],
}

/// Whether a node's nearest member ancestor (`found`, node rows) has one itself: an `inside`'s chains are longer than
/// its `nearest`'s.
fn nests(found: &[u32]) -> bool {
    use crate::code_rule_dsl::facts::nearest::NONE;
    found.iter().any(|&a| a != NONE && found[a as usize] != NONE)
}

fn slot(row: usize) -> (usize, usize) {
    let x = row / BASE + 1;
    let k = (usize::BITS - 1 - x.leading_zeros()) as usize;
    (k, row - BASE * ((1 << k) - 1))
}

impl Within {
    pub fn new(batch: Arc<Batch>, mode: Mode) -> Self {
        Within {
            mode,
            chunks: std::array::from_fn(|_| std::array::from_fn(|_| OnceLock::new())),
            made: Mutex::new(([0, 0], [FxHashMap::default(), FxHashMap::default()])),
            all: OnceLock::new(),
            zero: OnceLock::new(),
            shape: OnceLock::new(),
            len: AtomicUsize::new(0),
            members: FxHashMap::default(),
            kinds: Arc::from(Vec::new()),
            binding: crate::code_rule_dsl::facts::nearest::Binding::Cpu,
            expected: Mutex::new(0.0),
            chosen: std::sync::atomic::AtomicBool::new(false),
            solved: OnceLock::new(),
            solves: None,
            format: None,
            sets: (0..batch.docs.len()).map(|_| OnceLock::new()).collect(),
            shared: Mutex::new(FxHashMap::default()),
            batch,
        }
    }

    /// The same, its solve shared with the batch's other axes over its set (`Solves`).
    pub fn sharing(self, solves: Arc<Solves>) -> Self {
        Within { solves: Some(solves), ..self }
    }

    /// The same, solved where `binding` places it (`solved`).
    pub fn bound(self, binding: crate::code_rule_dsl::facts::nearest::Binding) -> Self {
        Within { binding, ..self }
    }

    /// The same, reading the documents' tapes in `format` rather than as held (a test's comparison of formats).
    pub fn reading(self, format: Format) -> Self {
        Within { format: Some(format), ..self }
    }

    /// `dt`'s tape: as held, or in the format a test asked for.
    fn tape_of<'d>(&self, dt: &'d DocTape) -> rerail_tape_format::tape::AnyTape<'d> {
        match self.format {
            Some(format) => dt.tape_in(format),
            None => dt.tape(),
        }
    }

    /// Document `doc`'s tape, which the tree's containment and the kind-set axes read (not spans).
    fn doc_tape(&self, doc: usize) -> Option<&DocTape> {
        if self.mode == Mode::Span {
            return None;
        }
        self.batch.docs[doc].facts.tape.as_deref()
    }

    /// The axis's set as the code numbers of `t`, document `doc`'s tape: the codes of a kind in it (every code for the
    /// tree's containment).
    fn set<T: Tape>(&self, doc: usize, t: &T) -> &CodeSet {
        self.sets[doc].get_or_init(|| {
            let tag = self.batch.docs[doc].tag;
            let key = (if self.mode == Mode::Tree { u16::MAX } else { u16::from(tag) }, std::ptr::from_ref(t.table()) as usize);
            let mut shared = self.shared.lock();
            shared
                .entry(key)
                .or_insert_with(|| {
                    if self.mode == Mode::Tree {
                        return Arc::new(t.table().set(|_| true));
                    }
                    // An ERROR node's kind is `ERROR_SYMBOL` (`Facts::kind`), slot 0 of its tag's stride whatever kind
                    // its entry holds: a member where the set names ERROR, as `member` and the device's bits read it
                    // (0289).
                    let members = self.members.get(&tag);
                    Arc::new(t.table().set(|e| {
                        let slot = if e.flags & rerail_tape_format::CODE_ERROR != 0 { 0 } else { usize::from(e.kind) };
                        members.is_some_and(|m| m.get(slot).is_some_and(|&x| x))
                    }))
                })
                .clone()
        })
    }

    /// Each node of document `doc`, in order, and whether it is in the set: off its tape, opened once.
    fn each_member(&self, doc: usize, mut f: impl FnMut(u32, bool)) {
        let p = &self.batch.docs[doc];
        match self.doc_tape(doc) {
            Some(dt) => with_tape!(self.tape_of(dt), |t| {
                let set = self.set(doc, &t);
                for (l, id) in p.nodes().map(|(l, x)| (l as usize, x)) {
                    f(l as u32, set.contains(t.code(id as usize)));
                }
            }),
            None => (0..p.len()).for_each(|l| f(l, self.member(doc, l))),
        }
    }

    /// `containers` on document `doc`'s tape `t`: the climb is the tape's parents, kept where the projection has them.
    fn containers_on<T: Tape>(&self, t: &T, doc: usize, p: &Projected, local: u32) -> Vec<u32> {
        let set = self.set(doc, t);
        let axis = match self.mode {
            Mode::Axis(axis) => Some(axis),
            _ => None,
        };
        let id = p.node(local) as usize;
        let mut out = Vec::new();
        if axis == Some(Axis::Has) && !set.contains(t.code(id)) {
            return out;
        }
        let mut up = t.parent(id);
        while let Some(a) = up {
            let l = p.local_of_node(a as u32);
            if l != NONE && (matches!(axis, None | Some(Axis::Has)) || set.contains(t.code(a))) {
                out.push(l);
                if axis == Some(Axis::Nearest) {
                    break;
                }
            }
            up = t.parent(a);
        }
        out
    }

    /// `contained` on document `doc`'s tape `t`: the subtree is the tape's interval, kept where the projection has it.
    fn contained_on<T: Tape>(&self, t: &T, doc: usize, p: &Projected, local: u32) -> Vec<u32> {
        let set = self.set(doc, t);
        let axis = match self.mode {
            Mode::Axis(axis) => Some(axis),
            _ => None,
        };
        let id = p.node(local) as usize;
        let mut out = Vec::new();
        if matches!(axis, Some(Axis::Nearest | Axis::Inside)) && !set.contains(t.code(id)) {
            return out;
        }
        let (mut n, end) = (id + 1, t.subtree_end(id));
        while n < end {
            let l = p.local_of_node(n as u32);
            if l != NONE {
                match axis {
                    None | Some(Axis::Inside) => out.push(l),
                    Some(Axis::Has) if set.contains(t.code(n)) => out.push(l),
                    Some(Axis::Has) => {}
                    // The subtree, less what lies below a nearer member: its nearest is that member or lower.
                    Some(Axis::Nearest) => {
                        out.push(l);
                        if set.contains(t.code(n)) {
                            n = t.subtree_end(n);
                            continue;
                        }
                    }
                }
            }
            n += 1;
        }
        out
    }

    /// The axis `axis` over the set of kind atoms `kinds`, its batch's solve run on `binding`. Nothing is made until
    /// a join reads it.
    pub fn axis(batch: Arc<Batch>, axis: Axis, kinds: &[i64], binding: crate::code_rule_dsl::facts::nearest::Binding) -> Self {
        use crate::code_rule_dsl::facts::core::{KIND_STRIDE, kind_atom};
        let kinds: rustc_hash::FxHashSet<i64> = kinds.iter().copied().collect();
        let members = batch
            .dialects
            .iter()
            .map(|(tag, d)| {
                let table = (0..KIND_STRIDE as usize)
                    .map(|code| kinds.contains(&i64::from(kind_atom(tag, d.canonical.get(code).copied().unwrap_or(code as u16)))))
                    .collect();
                (tag, table)
            })
            .collect();
        let mut sorted: Vec<i64> = kinds.into_iter().collect();
        sorted.sort_unstable();
        Within { members, kinds: Arc::from(sorted), binding, ..Within::new(batch, Mode::Axis(axis)) }
    }

    /// Whether the batch's solve answers the `n` side: an axis of ancestors in the set (`has` reads every ancestor),
    /// or containment in the tree, the ancestors of every kind (`inside` of the set of all kinds; decision 0277).
    fn solvable(&self) -> bool {
        matches!(self.mode, Mode::Axis(Axis::Nearest | Axis::Inside) | Mode::Tree)
    }

    /// Tests: whether the batch's solve has been made.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn is_solved(&self) -> bool {
        self.solved.get().is_some()
    }

    /// Tests: the rows made per looked-up node so far, both sides.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn made_rows(&self) -> usize {
        let made = self.made.lock();
        made.0[0] + made.0[1]
    }

    /// The first id of the rows made per looked-up `n`.
    fn looked_up(&self) -> usize {
        if self.solvable() { LOOKED_UP } else { 0 }
    }

    /// The batch's solve: every node's nearest strict ancestor in the set by the transducer's FIX, then per node its
    /// rows (that ancestor, or for `inside` the chain of them). Shared with the batch's other axes over its set
    /// (`Solves`): an `inside` whose set nests nowhere reads its `nearest`'s rows.
    fn solved(&self) -> &Solved {
        self.solved.get_or_init(|| {
            let Some(solves) = &self.solves else { return self.solve(None) };
            let slot = if matches!(self.mode, Mode::Axis(Axis::Inside)) { &solves.inside } else { &solves.nearest };
            Arc::clone(slot.get_or_init(|| self.solve(Some(&solves.nearest))))
        })
    }

    /// The solve made: for an `inside` given its set's `nearest` (`Solves`), that one where no member's nearest member
    /// ancestor has one (the chains are single), so both read one relation.
    fn solve(&self, nearest: Option<&OnceLock<Arc<Solved>>>) -> Arc<Solved> {
        let inside = matches!(self.mode, Mode::Axis(Axis::Inside) | Mode::Tree);
        let nearest = nearest.filter(|_| matches!(self.mode, Mode::Axis(Axis::Inside)));
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if let Some(s) = self.solved_in_chunk() {
            // The device writes the rows, not the chains: an `inside`'s rows hold its `nearest`'s, so as many rows are
            // the same rows.
            if let Some(n) = nearest.and_then(|n| n.get()).filter(|n| n.rows.len() == s.rows.len()) {
                return Arc::clone(n);
            }
            return Arc::new(s);
        }
        let found = self.found();
        if let Some(slot) = nearest
            && !nests(&found)
        {
            return Arc::clone(slot.get_or_init(|| Arc::new(self.solved_from(&found, false))));
        }
        Arc::new(self.solved_from(&found, inside))
    }

    /// Per node row its nearest strict ancestor in the set, a node row (`nearest::NONE` for none): on each document's
    /// tape, or by the transducer's FIX over the batch's parents.
    fn found(&self) -> Vec<u32> {
        use crate::code_rule_dsl::logic::profile;
        use crate::code_rule_dsl::facts::nearest::{self as file_local, Trees};
        let _span = profile::span(|| "kind-set axis solved for the batch".into());
        let n = self.batch.nodes().last().copied().unwrap_or(0);
        if self.binding == Binding::Cpu && (0..self.batch.docs.len()).all(|doc| self.doc_tape(doc).is_some()) {
            profile::work(profile::Unit::Node, n as u64);
            return self.found_on_tapes(n);
        }
        let (mut parent, mut kind) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for (doc, p) in self.batch.docs.iter().enumerate() {
            let base = self.batch.nodes()[doc] as u32;
            for l in 0..p.len() {
                let up = p.parent(l as usize);
                parent.push(if up == NONE { file_local::NONE } else { base + up });
            }
            self.each_member(doc, |_, m| kind.push(i64::from(m)));
        }
        profile::work(profile::Unit::Node, n as u64);
        // Membership is the kind: the set is `{1}`.
        let trees = Trees { parent: &parent, kind: &kind };
        file_local::nearest(&trees, &[vec![1]], self.binding)
            .or_else(|_| file_local::nearest(&trees, &[vec![1]], Binding::Cpu))
            .expect("the CPU's nearest pass")
            .pop()
            .unwrap_or_default()
    }

    /// Per node row its rows: its nearest member ancestor (`found`, a node row), or for `inside` the chain of them;
    /// counted, then written once as ids into the axis's relation (decision 0353 step 5).
    fn solved_from(&self, found: &[u32], inside: bool) -> Solved {
        use crate::code_rule_dsl::facts::nearest as file_local;
        use crate::code_rule_dsl::facts::rows::Part;
        let n = found.len();
        let chain = |v: usize, each: &mut dyn FnMut(u32)| {
            let mut a = found[v];
            while a != file_local::NONE {
                each(a);
                a = if inside { found[a as usize] } else { file_local::NONE };
            }
        };
        let mut start = Vec::with_capacity(n + 1);
        let mut len = 0u32;
        for v in 0..n {
            start.push(len);
            chain(v, &mut |_| len += 1);
        }
        start.push(len);
        // A derived fact like a rule's head, in a run's block alone, with room for an index by each key a stratum
        // probes it by (`n`, `a`, and both in either order: `AXIS_KEYS`), as the device's solve writes it. File-local: a row's `n` and `a` are of
        // one document, so each column holds its ids' low bits, their key a run's, one run a document that has rows
        // (decision 0494 step 7).
        let offsets = self.batch.nodes();
        let first = |doc: usize| start[offsets[doc]];
        let has_rows = |doc: usize| first(doc + 1) > first(doc);
        let runs = (0..self.batch.docs.len()).filter(|&d| has_rows(d)).count();
        let (rows, keys) = Part::over_key_runs(crate::code_rule_dsl::facts::rows::Life::Run, 2, len as usize, crate::code_rule_dsl::facts::rows::AXIS_KEYS, runs);
        for (k, (doc, &key)) in self.batch.docs.keys().iter().enumerate().filter(|&(d, _)| has_rows(d)).enumerate() {
            keys.put(k, first(doc), key);
        }
        let mut at = 0u32;
        for v in 0..n {
            let id = self.batch.id(v);
            chain(v, &mut |a| {
                // SAFETY: row `at` is below the `len` rows reserved, and nothing reads it before `written`.
                unsafe {
                    rows.put(at, 0, id);
                    rows.put(at, 1, self.batch.id(a as usize));
                }
                at += 1;
            });
        }
        // SAFETY: rows `0..len` are written.
        unsafe { rows.written(len) };
        Solved {
            start,
            rows,
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            start_in_chunk: None,
        }
    }

    /// Each node row's nearest member ancestor on its document's tape: one pass of the set's matches (`nearest_all`),
    /// kept where the projection has them, mapped to node rows.
    fn found_on_tapes(&self, n: usize) -> Vec<u32> {
        use crate::code_rule_dsl::facts::nearest as file_local;
        let mut found = Vec::with_capacity(n);
        let mut near = Vec::new();
        for (doc, p) in self.batch.docs.iter().enumerate() {
            let base = self.batch.nodes()[doc] as u32;
            let dt = self.doc_tape(doc).expect("every document holds its tape");
            with_tape!(self.tape_of(dt), |t| {
                nearest_all(&t, self.set(doc, &t), |id| p.local_of_node(id as u32) != NONE, &mut near);
            });
            found.extend(p.nodes().map(|(_, id)| match near[id as usize] {
                rerail_tape_format::tape::NONE => file_local::NONE,
                a => base + p.local_of_node(a),
            }));
        }
        found
    }

    /// The solve's first row per node row (`nodes + 1` values).
    fn starts(&self) -> &[u32] {
        let s = self.solved();
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if let Some(at) = s.start_in_chunk {
            let n = self.batch.nodes().last().copied().unwrap_or(0) + 1;
            // SAFETY: the device wrote `n` values there, and the chunk lives as long as the batch this axis holds.
            return unsafe { std::slice::from_raw_parts(self.batch.chunk().block.ptr::<u32>(at), n) };
        }
        &s.start
    }

    /// Whether the axis is solved on the device: the binding is the device's and a solve is registered.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn on_device(&self) -> bool {
        self.binding == Binding::Metal && crate::code_rule_dsl::facts::nearest::chunk_axis().is_some()
    }

    /// The solve's rows: the axis's relation.
    fn solved_rows(&self) -> &crate::code_rule_dsl::facts::rows::Part {
        &self.solved().rows
    }

    /// The solve made on the device in the batch's chunk (decision 0289 §2), when the binding is the device's and
    /// one is registered: `None` sends it to the host's.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn solved_in_chunk(&self) -> Option<Solved> {
        // The binding first: the batch's chunk block is the device's (its tapes in one block), made on first use and
        // kept by the batch, so a CPU run that asked for it held the whole corpus's (6.2 GB of the driver's memory on
        // 3,089 files, `rf_crossing`'s solve) to solve on the host anyway.
        if !self.on_device() {
            return None;
        }
        self.solved_in(&[self.batch.chunk()]).map(|c| Solved { start: Vec::new(), rows: c.rows, start_in_chunk: Some(c.start) })
    }

    /// Its solve and those of `more`, the axis over a chunk's tape values (decision 0431 §8), made on the device in
    /// one relation: its rows, then each one's of `more`, as the device binds a layered source's (`values::Layered`).
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    pub(super) fn solved_with(&self, more: &[&Within]) -> Option<crate::code_rule_dsl::facts::rows::Part> {
        if !self.solvable() || !self.on_device() || more.iter().any(|w| w.mode != self.mode) {
            return None;
        }
        let chunks: Vec<&super::device_chunk::Chunk> = std::iter::once(self).chain(more.iter().copied()).map(|w| w.batch.chunk()).collect();
        self.solved_in(&chunks).map(|c| c.rows)
    }

    /// The axis solved on the device over `chunks`, in one relation (`nearest::ChunkAxis`).
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn solved_in(&self, chunks: &[&super::device_chunk::Chunk]) -> Option<crate::code_rule_dsl::facts::nearest::InChunk> {
        use crate::code_rule_dsl::facts::core::KIND_STRIDE;
        use crate::code_rule_dsl::facts::nearest::chunk_axis;
        let solve = chunk_axis().filter(|_| self.on_device())?;
        let _span = crate::code_rule_dsl::logic::profile::span(|| "kind-set axis solved on the device in the chunk".into());
        // Per tag and slot a bit: the host's `member`, ERROR at slot 0 (`kind_atom`), as the kernel reads an atom.
        let bits = (self.mode != Mode::Tree).then(|| {
            let mut bits = vec![0u32; 256 * KIND_STRIDE as usize / 32];
            for (&tag, table) in &self.members {
                for (code, _) in table.iter().enumerate().filter(|&(_, &m)| m) {
                    let at = usize::from(tag) * KIND_STRIDE as usize + code;
                    bits[at / 32] |= 1 << (at % 32);
                }
            }
            bits
        });
        let inside = matches!(self.mode, Mode::Axis(Axis::Inside) | Mode::Tree);
        match solve(chunks, bits.as_deref(), inside) {
            Ok(c) => Some(c),
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("kind-set axes (decision 0289)", || format!("solved on the host: {why}"));
                None
            }
        }
    }

    /// The planner expects `keys` more lookups of `n`: the batch's solve is chosen once they cost more than it.
    pub(super) fn expect(&self, keys: f64) {
        // Containment's solve is every ancestor of every node: only a reader of the whole relation chooses it.
        if !self.solvable() || self.mode == Mode::Tree || self.chosen.load(Ordering::Relaxed) {
            return;
        }
        let mut expected = self.expected.lock();
        *expected += keys.max(0.0);
        let nodes = self.batch.nodes().last().copied().unwrap_or(0) as f64;
        if *expected * LOOKUP_NS > nodes * SOLVE_NS {
            self.chosen.store(true, Ordering::Relaxed);
            crate::code_rule_dsl::logic::profile::decision("kind-set axes (decision 0143)", || {
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
        if self.mode == Mode::Tree {
            return true;
        }
        let p = &self.batch.docs[doc];
        // The table's slot is the atom's place in its stride: ERROR's atom is slot 0 (`kind_atom`), as on the tape and
        // in the device's bits. `kind_code` gives ERROR as `u16::MAX`, past the table, which left it out of every set.
        let slot = p.facts.kind(p.node(local)) % crate::code_rule_dsl::facts::core::KIND_STRIDE;
        self.members.get(&p.tag).is_some_and(|t| t.get(slot as usize).is_some_and(|&m| m))
    }

    /// The document and local node of node row `row`.
    fn at(&self, row: u32) -> (usize, u32) {
        let doc = self.batch.nodes().partition_point(|&o| o <= row as usize) - 1;
        (doc, row - self.batch.nodes()[doc] as u32)
    }

    fn span(&self, doc: usize, local: u32) -> (u32, u32) {
        let p = &self.batch.docs[doc];
        let id = p.node(local) as usize;
        (p.facts.start(id as u32), p.facts.end(id as u32))
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
        if let Some(dt) = self.doc_tape(doc) {
            return with_tape!(self.tape_of(dt), |t| self.containers_on(&t, doc, p, local));
        }
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
        if let Some(dt) = self.doc_tape(doc) {
            return with_tape!(self.tape_of(dt), |t| self.contained_on(&t, doc, p, local));
        }
        if let Mode::Axis(axis) = self.mode {
            let end = local + p.size(local as usize);
            return match axis {
                _ if axis != Axis::Has && !self.member(doc, local) => Vec::new(),
                Axis::Inside => (local + 1..=end).collect(),
                Axis::Has => (local + 1..=end).filter(|&n| self.member(doc, n)).collect(),
                Axis::Nearest => {
                    // The subtree, less what lies below a nearer member: its nearest is that member or lower.
                    let (mut out, mut n) = (Vec::new(), local + 1);
                    while n <= end {
                        out.push(n);
                        n += if self.member(doc, n) { p.size(n as usize) + 1 } else { 1 };
                    }
                    out
                }
            };
        }
        if self.mode == Mode::Tree {
            return (local + 1..=local + p.size(local as usize)).collect();
        }
        let (s, e) = self.span(doc, local);
        let end = local + p.size(local as usize);
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

    /// Whether node `n` is strictly below node `a` (ids): in one document, `n` in `a`'s preorder interval of the
    /// projection (`size`, a rank of the held tape's subtree end), the interval the made rows are (`contained`).
    /// Nothing is made, and no tape reader is opened: a reader a call (`tape_of`) cost more than the rows it saved
    /// in strata that check the same nodes again (t50 step 4).
    fn below(&self, n: i64, a: i64) -> bool {
        let (Some((dn, ln)), Some((da, la))) = (self.batch.node(n), self.batch.node(a)) else { return false };
        dn == da && la < ln && ln <= la + self.batch.docs[da].size(la as usize)
    }

    /// The rows of node row `row` as inner (`side` 0) or outer (1), made on the first lookup.
    fn rows(&self, side: usize, row: u32) -> (u32, u32) {
        if let Some(&run) = self.made.lock().1[side].get(&row) {
            return run;
        }
        let (doc, local) = self.at(row);
        let base = self.batch.nodes()[doc] as u32;
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

    /// A row made per looked-up node (not the solve's): inner node row << 32 | outer node row.
    fn pair(&self, row: usize) -> u64 {
        debug_assert!(!(self.solvable() && row < LOOKED_UP), "a solved row is the relation's");
        let side = usize::from(row >= OUTER);
        let (k, at) = slot(row - if side == 0 { self.looked_up() } else { OUTER });
        self.chunks[side][k].get().unwrap_or_else(|| panic!("within: row {row} not made (mode {:?}, solvable {}, solved {}, {} made)", self.mode, self.solvable(), self.solved.get().is_some(), self.made.lock().0[side]))[at].load(Ordering::Relaxed)
    }

    fn shape(&self) -> Shape {
        *self.shape.get_or_init(|| {
            if let Mode::Axis(axis) = self.mode {
                return self.axis_shape(axis);
            }
            let (mut rows, mut nodes, mut ancestors, mut sq_inner) = (0i64, 0i64, 0i64, 0i64);
            for p in &self.batch.docs {
                for (l, _, depth) in p.places() {
                    // Span mode counts the node itself on both sides.
                    let own = i64::from(self.mode == Mode::Span);
                    let (d, s) = (i64::from(depth) + own, i64::from(p.size(l as usize)) + own);
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
        let mut c = AxisCounts::default();
        for doc in 0..self.batch.docs.len() {
            c.add(self.axis_counts(doc));
        }
        let AxisCounts { members, covered, below, below_sq, above, above_sq, nodes } = c;
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

    /// Document `doc`'s part of `axis_shape`'s counts: one walk of its nodes.
    fn axis_counts(&self, doc: usize) -> AxisCounts {
        let p = &self.batch.docs[doc];
        let mut c = AxisCounts::default();
        // The end of the outermost member's subtree the walk is in.
        let mut until: Option<u32> = None;
        self.each_member(doc, |l, member| {
            c.nodes += 1;
            if until.is_some_and(|u| l <= u) {
                c.covered += 1;
            }
            if !member {
                return;
            }
            let size = p.size(l as usize);
            if !until.is_some_and(|u| l <= u) {
                until = Some(l + size);
            }
            let (s, d) = (i64::from(size), i64::from(p.depth(l as usize)));
            c.members += 1;
            c.below += s;
            c.below_sq += s * s;
            c.above += d;
            c.above_sq += d * d;
        });
        c
    }

    /// Whether the relation has no row, found without counting them all: an axis's documents are walked until one
    /// holds a row (a native stratum asks this of each relation its rules join, and counted, every document's every
    /// containment pair was walked; 14 ms of a warm run that ran no rule).
    fn holds_no_row(&self) -> bool {
        if self.solvable() {
            if self.solved.get().is_some() {
                return self.solved_rows().len() == 0;
            }
            if let Some(shape) = self.shape.get() {
                return shape.rows == 0;
            }
            if let Mode::Axis(axis) = self.mode {
                return (0..self.batch.docs.len()).all(|doc| self.axis_counts(doc).rows(axis) == 0);
            }
            return self.shape().rows == 0;
        }
        self.len() == 0
    }
}

/// An axis's counts over documents (`Within::axis_counts`): its members, the nodes inside a member's subtree, the
/// members' subtree sizes and depths (and their squares), and the nodes walked.
#[derive(Clone, Copy, Default)]
struct AxisCounts {
    members: i64,
    covered: i64,
    below: i64,
    below_sq: i64,
    above: i64,
    above_sq: i64,
    nodes: i64,
}

impl AxisCounts {
    fn add(&mut self, o: AxisCounts) {
        self.members += o.members;
        self.covered += o.covered;
        self.below += o.below;
        self.below_sq += o.below_sq;
        self.above += o.above;
        self.above_sq += o.above_sq;
        self.nodes += o.nodes;
    }
    /// The axis's rows of these counts (`axis_shape`'s `rows`).
    fn rows(&self, axis: Axis) -> i64 {
        match axis {
            Axis::Nearest => self.covered,
            Axis::Inside => self.below,
            Axis::Has => self.above,
        }
    }
}

impl Source for Within {
    /// A document's rows are its nodes' (an axis never leaves its document), in the row space `value` reads: the
    /// batch's solve, where each node row's pairs are its run (decision 0494). The span mode's rows are made by lookup
    /// and have no such space: `None`.
    fn rows_of_documents(&self, docs: &std::collections::BTreeSet<u32>) -> Option<Vec<usize>> {
        if !self.solvable() {
            return None;
        }
        let (start, nodes) = (self.starts(), self.batch.nodes());
        Some(docs.iter().filter_map(|&k| self.batch.docs.position(k)).flat_map(|i| start[nodes[i]] as usize..start[nodes[i + 1]] as usize).collect())
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    /// Read whole (the device, a whole read's own index, on every target): the batch's solve, the relation it was
    /// written into, where it lies (decision 0353 step 5). An axis with no solve has none: its rows are made per
    /// looked-up node.
    fn in_block(&self) -> Option<crate::code_rule_dsl::facts::rows::InBlock> {
        self.solvable().then(|| self.solved_rows().in_block(self.solved_rows().len()))
    }
    fn columns(&self) -> Vec<String> {
        self.names().iter().map(|&c| c.to_owned()).collect()
    }
    /// Every pair once: a scan makes each node's containers, ids `0..len`. A solvable axis's are its solve's pairs,
    /// counted exactly from the tree (`shape`) until something reads them: a guest round digests each input by its
    /// length, and the solve made for that count alone held 780 MB of a PostHog run's 2.5 GB peak.
    fn len(&self) -> usize {
        if self.solvable() {
            if self.solved.get().is_some() {
                return self.solved_rows().len() as usize;
            }
            return self.shape().rows as usize;
        }
        *self.all.get_or_init(|| {
            for row in 0..self.batch.nodes().last().copied().unwrap_or(0) as u32 {
                self.rows(0, row);
            }
            self.len.load(Ordering::Acquire)
        })
    }
    fn estimated_len(&self) -> usize {
        self.shape().rows as usize
    }
    fn holds_none(&self) -> bool {
        self.holds_no_row()
    }
    /// By `n`: its containers listed up the tree, 5.5 derived lookups; a kind-set axis's, 3 (decision 0312). By
    /// `a`: the solved pairs searched, one.
    fn lookup_cost(&self, column: usize) -> f64 {
        match (self.mode, column) {
            (Mode::Axis(_), 0) => 3.0,
            (Mode::Tree | Mode::Span, 0) => 5.5,
            _ => 1.0,
        }
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
    /// `has`, read by the device off the tape (decision 0353 step 5): where the batch's views are read in place, its
    /// `node` view, which a stage's walk reads a node's kind, ancestors and subtree through, and the set's atoms.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn has_walk(&self) -> Option<crate::code_rule_dsl::facts::vtab::HasWalk> {
        use super::device_chunk::{VIEW_NODE, read_kind, views_in_place};
        if self.mode != Mode::Axis(Axis::Has) {
            return None;
        }
        let chunk = self.batch.chunk();
        let v = views_in_place(&self.batch, chunk)?;
        let mut view = crate::code_rule_dsl::facts::vtab::ViewInPlace { dir: v.dir, kind: read_kind(VIEW_NODE), buffers: v.buffers.clone(), cache: v.cache.clone(), failed: 0, format: v.format, keep: Arc::clone(&self.batch) as Arc<dyn std::any::Any + Send + Sync> };
        view.failed = view.cache_failed();
        Some(crate::code_rule_dsl::facts::vtab::HasWalk { nodes: self.batch.nodes().last().copied().unwrap_or(0), view, kinds: Arc::clone(&self.kinds) })
    }
    /// A kind-set axis or containment read whole (the device's keyed read, decision 0277): the batch's solve
    /// (decision 0143), one pass over the nodes, chosen from here on, so its `n` lookups read the same rows. Spans
    /// have no such solve.
    fn complete_rows(&self) -> bool {
        if !self.solvable() {
            return false;
        }
        if !self.chosen.swap(true, Ordering::Relaxed) {
            crate::code_rule_dsl::logic::profile::decision("kind-set axes (decision 0143)", || "solved for the batch: a stratum on the device reads the axis whole (decision 0277)".into());
        }
        true
    }
    /// By `n`, a document's nodes looked up: a node's pairs are its own document's (none crosses documents), so
    /// the rows of the documents `owned` names are found without the batch's solve or a scan.
    fn rows_owned(&self, g: usize, owned: &dyn Fn(i64) -> bool) -> Option<Vec<usize>> {
        if g != 0 {
            return None;
        }
        let mut out = Vec::new();
        for doc in 0..self.batch.docs.len() {
            let (from, to) = (self.batch.nodes()[doc], *self.batch.nodes().get(doc + 1)?);
            if from == to || !owned(self.batch.id(from)) {
                continue;
            }
            let ids: Vec<i64> = (from..to).map(|r| self.batch.id(r)).collect();
            out.extend(self.lookup(0, &ids)?);
        }
        Some(out)
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        if column == 0 {
            self.expect(keys);
        }
    }
    /// `within(n, a)` with both given is a test, not a join (t50 step 4): `lookup_all` reads `n`'s rows (made, or the
    /// batch's solve) for the one of them that is `a`, and `holds` reads the projection's interval of `a` (`below`).
    fn composite(&self) -> bool {
        self.mode == Mode::Tree
    }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        let (Some(&(_, n)), Some(&(_, a))) = (equal.iter().find(|e| e.0 == 0), equal.iter().find(|e| e.0 == 1)) else { return None };
        if self.mode != Mode::Tree || equal.iter().any(|&(c, v)| v != if c == 0 { n } else { a }) {
            return None;
        }
        Some(self.lookup(0, &[n])?.into_iter().filter(|&r| self.value(r, 1) == a).collect())
    }
    fn holds(&self, equal: &[(usize, i64)]) -> Option<bool> {
        if self.mode != Mode::Tree {
            return None;
        }
        let (Some(&(_, n)), Some(&(_, a))) = (equal.iter().find(|e| e.0 == 0), equal.iter().find(|e| e.0 == 1)) else { return None };
        if equal.iter().any(|&(c, v)| v != if c == 0 { n } else { a }) {
            return Some(false);
        }
        Some(self.below(n, a))
    }
    /// By `n` on the batch's solve: each of the node's rows with its values, read from the solve's rows as they are
    /// found. No list of rows is made, and no row's value is read again (`lookup` then `value`). An axis not solved
    /// for the batch gives nothing here (`false`), and its rows are made per lookup as before.
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        if column != 0 || !self.reads_solved() || columns.len() > 2 {
            return false;
        }
        let Some((doc, local)) = self.batch.node(value) else { return true };
        let row = self.batch.nodes()[doc] + local as usize;
        let start = self.starts();
        let rows = self.solved_rows();
        let mut got = [0i64; 2];
        for r in start[row]..start[row + 1] {
            for (j, &c) in columns.iter().enumerate() {
                got[j] = rows.get(r, c.min(1));
            }
            each(r as usize, &got[..columns.len()]);
        }
        true
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut out = Vec::new();
        for &v in values {
            let Some((doc, local)) = self.batch.node(v) else { continue };
            let row = self.batch.nodes()[doc] + local as usize;
            if column == 0 && self.reads_solved() {
                let start = self.starts();
                out.extend(start[row] as usize..start[row + 1] as usize);
                continue;
            }
            // By `a`, where the rows a scan reads are made (the batch's solve, or every node's containers), its
            // rows are those: in each contained node's run, the pair whose outer node is `a`. A lookup's rows are
            // then the scan's, as a reader of both (a relation's version, `Before`) takes them.
            if column == 1 && (self.solved.get().is_some() || self.all.get().is_some()) {
                let base = self.batch.nodes()[doc];
                let solved = self.solved.get().is_some();
                for c in self.contained(doc, local) {
                    let inner = (base + c as usize) as u32;
                    if solved {
                        // The solve's relation holds ids (decision 0353 step 5): the row whose outer id is `a`'s.
                        let (start, rows) = (self.starts(), self.solved_rows());
                        out.extend((start[inner as usize]..start[inner as usize + 1]).filter(|&r| rows.get(r, 1) == v).map(|r| r as usize));
                    } else {
                        let (from, n) = self.rows(0, inner);
                        out.extend((from as usize..(from + n) as usize).filter(|&r| self.pair(r) & 0xffff_ffff == row as u64));
                    }
                }
                continue;
            }
            let (start, n) = self.rows(column.min(1), row as u32);
            out.extend(start as usize..(start + n) as usize);
        }
        Some(out)
    }
    /// A pair's node's document: the key of the document its row is in (`Batch::locate`), no id read.
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        if self.solvable() {
            // The solve's rows are ids: each one's document is its node's.
            let rows = self.solved_rows();
            for r in 0..rows.len() {
                let id = rows.get(r, column.min(1));
                let key = self.batch.node(id).map_or(0, |(doc, _)| self.batch.docs[doc].key);
                each(r as usize, if key == 0 { id } else { i64::from(key) });
            }
            return;
        }
        for r in 0..self.len() {
            let pair = self.pair(r);
            let node = if column == 0 { (pair >> 32) as usize } else { (pair & 0xffff_ffff) as usize };
            let p = self.batch.locate(View::Node, node).0;
            // A document keyed 0 has ids below `1 << 32`, each its own "document" (`memo::document_of`).
            each(r, if p.key == 0 { self.value(r, column) } else { i64::from(p.key) });
        }
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        if self.solvable() && row < LOOKED_UP {
            return self.solved_rows().get(row as u32, column.min(1));
        }
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
