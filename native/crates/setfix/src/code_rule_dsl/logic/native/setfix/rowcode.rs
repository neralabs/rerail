//! A join step's filters as one row program (decision 0095 §2).
//!
//! An atom that binds columns is often followed by steps that only test or compute from what is
//! bound: `lt`, `eq`, `ne`, `in`, `deq`/`dne`, `docof`, `let`. Those steps are fused with the atom
//! (`fuse`). The atom's candidates are gathered a chunk at a time into columns, and the steps run
//! over the chunk as one block program of the transducer's `Program` (`block::BlockProgram` with
//! `rows`): each test a `Cmp` mask, joined by the mask operations. Its extract pass gives the rows
//! that pass all of them, and only those go on to the next step.
//!
//! What each step reads becomes one of the row program's inputs:
//! - a slot the atom binds is a column of the chunk;
//! - a slot bound before the atom, or a constant, is a parameter, one value for the whole chunk;
//! - `@doc(x)` (`docof`, `deq`, `dne`) is computed: `x >> 32`. Computed values are the plan's own
//!   expression list (`Word`), evaluated over the chunk's columns before the program runs. They
//!   do not fit `Program`, which holds no word-valued row code (decision 0095 §2).
//!
//! - a test (an atom of an input that binds nothing: `rr_ast_8(n)`, `!binder(n)`) whose keys read the
//!   chunk's columns is computed too: the run asks the input for each row still passing the tests
//!   before it (`Fused::tests`), its verdicts a column (`Word::Held`), 1 where the row passes. The
//!   shape kernel of a scan and the tests on its nodes (the design's layer 5): the chunk's tests are
//!   asked one input at a time, not a row at a time through the walk. A test of a node by a view keyed by it
//!   (`kind(n, #k)` after the atom that binds `n`, t57 E6) is asked of those rows at once and in place
//!   (`Test::view`, `Source::read_own_batch`: the rows' nodes read as one batch, every node found first, then the
//!   tested column read and compared).
//!
//! The steps' observed fanouts (`JoinRule::counts`) stay exact: the program outputs each fused
//! step's running mask, and its popcount is the rows that step passed on.

use rerail_blocks::frame::InlineVec;
use rerail_blocks::vector::Vector;
use super::{Key, ScalarKind, Src, Step};
use crate::code_rule_dsl::facts::vtab::{ProbeCursor, Source};
use std::sync::Arc;
use rerail_compiler::block::{BlockOp, BlockProgram, CmpOp, Rhs, RowBatch, RowShape};

/// The rows gathered before the program runs over them.
pub(super) const CHUNK: usize = 1024;

/// A value of the chunk's rows: an atom column, or a value computed from one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    /// The atom's column.
    Col(usize),
    /// The atom's column's document: `@doc(x)`, `x >> 32`.
    Doc(usize),
    /// Test `k`'s verdict on the row (`Fused::tests`): 1 where it passes (a negated one, where its input holds no
    /// row), else 0.
    Held(usize),
    /// Lookup test `k`'s (`ViewTest::looks`) value of its input's column `c` for the row: the one row its key names,
    /// read with the chunk's others (`Source::read_own_batch`).
    Look(usize, usize),
}

/// A test fused as a computed column: the step (after the atom) whose atom it asks, and each slot its keys read with
/// where its value is: a program column (the row's), or a parameter (a slot bound before the atom, or a constant,
/// that a fused `let` put in another slot, which the walk sets only for a row that passes).
#[derive(Clone)]
pub struct Test {
    pub at: usize,
    pub neg: bool,
    pub slots: Vec<(usize, Bind)>,
    /// Where the input is a view that reads the chunk's rows at once (t57 E6, decision 0706): asked so, for the rows
    /// still passing, not row by row.
    pub view: Option<ViewTest>,
}

/// A view's step asked a chunk at a time (`Test::view`): a node's own row by its id (`kind(n, #k)`, or a lookup,
/// `setfix::lookups`, `kind(n, k)` binding `k`), or a pair (`within(n, f)` with both bound).
///
/// A node's row: the node (`node`, a row's column or a parameter) is the source's column `by`, the column it reads
/// its rows by their own id in; the chunk's rows still passing are read as one batch (`Source::read_own_batch`) of
/// columns `reads`, each `checks` read equal to its key's value, each `equal` pair of reads equal, and each of `looks`
/// (the atom's column, its read) the value a lookup binds (`Word::Look`).
///
/// A pair: `pair`'s sides (a row's column or a parameter, at least one a row's) are the source's columns `by` and
/// `col`, the chunk's pairs asked at once (`Source::holds_batch`, decision 0706 E5).
#[derive(Clone)]
pub struct ViewTest {
    pub src: Arc<dyn Source>,
    pub node: PairKey,
    pub by: usize,
    pub reads: Vec<usize>,
    pub checks: Vec<(usize, PairKey)>,
    pub equal: Vec<(usize, usize)>,
    pub looks: Vec<(usize, usize)>,
    pub col: usize,
    pub pair: Option<[PairKey; 2]>,
}

/// A value a fused view step reads (`ViewTest`): a program column, or a parameter.
#[derive(Clone, Copy)]
pub enum PairKey {
    Col(usize),
    Param(usize),
}

/// A derived input's view, as a fused test reads it: its source and, by the atom's column, the source's.
pub type ViewOf<'a> = &'a dyn Fn(&Src) -> Option<(Arc<dyn Source>, Vector<usize>)>;

/// A value the same for every row of a chunk: a slot bound before the atom, or a constant, or
/// its document.
#[derive(Clone, Copy)]
pub struct Param {
    pub key: Key,
    pub doc: bool,
}

/// A slot's value within the fused steps.
#[derive(Clone, Copy)]
enum Val {
    Row(Word),
    Param(Param),
}

/// An atom fused with the steps after it.
#[derive(Clone)]
pub struct Fused {
    /// The steps fused after the atom.
    pub steps: usize,
    pub program: BlockProgram,
    /// The program's columns, each a `Word` of the atom's rows; the raw ones first are gathered,
    /// the computed ones are evaluated from them.
    pub columns: Vec<Word>,
    pub params: Vec<Param>,
    /// For each fused step, the program output whose rows it passed on (`None`: every row).
    pub passed: Vec<Option<usize>>,
    /// The output whose rows go on.
    pub last: usize,
    /// The slots set for each row that goes on, from the program's columns or parameters.
    pub binds: Vec<(usize, Bind)>,
    /// The gathered columns: (program column, atom column).
    pub(super) gather: Vec<(usize, usize)>,
    /// The tests asked of an input, in step order (`Word::Held`).
    pub tests: Vec<Test>,
}

#[derive(Clone, Copy)]
pub enum Bind {
    Column(usize),
    Param(usize),
}

/// Program assembly: the inputs interned, the operations in order.
#[derive(Default)]
struct Build {
    columns: Vec<Word>,
    params: Vec<Param>,
    ops: Vec<BlockOp>,
}

impl Build {
    fn column(&mut self, w: Word) -> u16 {
        let at = self.columns.iter().position(|&c| c == w).unwrap_or_else(|| {
            self.columns.push(w);
            self.columns.len() - 1
        });
        at as u16
    }

    fn param(&mut self, p: Param) -> u16 {
        let same = |q: &Param| q.doc == p.doc && match (q.key, p.key) {
            (Key::Slot(a), Key::Slot(b)) => a == b,
            (Key::Const(a), Key::Const(b)) => a == b,
            _ => false,
        };
        let at = self.params.iter().position(same).unwrap_or_else(|| {
            self.params.push(p);
            self.params.len() - 1
        });
        at as u16
    }

    fn op(&mut self, op: BlockOp) -> u16 {
        self.ops.push(op);
        (self.ops.len() - 1) as u16
    }

    /// `a op b` as a mask, `None` when neither side is a row's (a test of parameters alone,
    /// which the fusion leaves to the step).
    fn compare(&mut self, a: Val, op: CmpOp, b: Val) -> Option<u16> {
        // `b < a` is `!(a <= b)`, and `b <= a` is `!(a < b)`.
        let (col, op, rhs, not) = match (a, b) {
            (Val::Row(x), Val::Row(y)) => (self.column(x), op, Rhs::Col(self.column(y)), false),
            (Val::Row(x), Val::Param(p)) => (self.column(x), op, Rhs::Param(self.param(p)), false),
            (Val::Param(p), Val::Row(y)) => {
                let rhs = Rhs::Param(self.param(p));
                match op {
                    CmpOp::Eq | CmpOp::Ne => (self.column(y), op, rhs, false),
                    CmpOp::Lt => (self.column(y), CmpOp::Le, rhs, true),
                    CmpOp::Le => (self.column(y), CmpOp::Lt, rhs, true),
                }
            }
            (Val::Param(_), Val::Param(_)) => return None,
        };
        let m = self.op(BlockOp::Cmp { col, op, rhs, width: 64 });
        Some(if not { self.op(BlockOp::Not(m)) } else { m })
    }
}

/// The fusions of a rule's steps, by the step of the atom: each atom that binds a slot, with the
/// row-code steps after it, when at least one of them tests. None after a step that can leave a
/// sum pending (the run checks each step for it, which fused steps would skip).
/// A lookup (`lookups`, `setfix::lookups`) is no atom's scan: its one row is read, by the walk or, after a fused atom,
/// with the chunk's others as its computed columns.
pub(super) fn fuse(steps: &[Step], lookups: &[bool], view: ViewOf<'_>) -> Vec<Option<Box<Fused>>> {
    let mut out: Vec<Option<Box<Fused>>> = (0..steps.len()).map(|_| None).collect();
    for (i, step) in steps.iter().enumerate() {
        if steps[..i].iter().any(|s| matches!(s, Step::Scalar { kind: ScalarKind::Sum, bind: true, .. })) {
            break;
        }
        if let Step::Atom { neg: false, binds, .. } = step
            && !binds.is_empty()
            && !lookups.get(i).copied().unwrap_or(false)
        {
            out[i] = fused_viewing(binds, &steps[i + 1..], tests_on(), lookups.get(i + 1..).unwrap_or(&[]), view).map(Box::new);
        }
    }
    out
}

/// Whether a test of an input is fused into its scan's chunk (`Test`): `RERAIL_FUSED_TESTS=off` (a measurement aid,
/// the run's setting) leaves it to the walk, as before.
fn tests_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_FUSED_TESTS").map_or(true, |v| v != "off"))
}

/// `fused_viewing` with no view's tests (a reader that has none, the device's): its lookups (`lookups`, per step of
/// `after`) end the fusion.
pub fn fused(binds: &[(usize, usize)], after: &[Step], ask: bool, lookups: &[bool]) -> Option<Fused> {
    fused_viewing(binds, after, ask, lookups, &|_| None)
}

/// The steps after an atom that binds `binds` fused with it, as far as each is row code: a comparison, a document's
/// test or read, a `let`, a set's test, a test of an input (`Test`), or a lookup (`lookups`, per step of `after`) a
/// view reads with the chunk's others, its values computed columns. `ask`: whether a test of an input is asked row
/// by row (which only the CPU's walk does); a view's step read a chunk at a time (`view`) is fused either way.
pub fn fused_viewing(binds: &[(usize, usize)], after: &[Step], ask: bool, lookups: &[bool], view: ViewOf<'_>) -> Option<Fused> {
    let mut env: Vec<(usize, Val)> = binds.iter().map(|&(c, s)| (s, Val::Row(Word::Col(c)))).collect();
    let mut b = Build::default();
    let mut acc: Option<u16> = None;
    let mut passed: Vec<Option<u16>> = Vec::new();
    let mut steps = 0;
    let mut tests: Vec<Test> = Vec::new();
    for (at, step) in after.iter().enumerate() {
        let val = |env: &[(usize, Val)], k: Key| -> Val {
            match k {
                Key::Slot(s) => env.iter().rev().find(|e| e.0 == s).map_or(Val::Param(Param { key: k, doc: false }), |e| e.1),
                Key::Const(_) => Val::Param(Param { key: k, doc: false }),
            }
        };
        let doc = |v: Val| -> Option<Val> {
            match v {
                Val::Row(Word::Col(c)) => Some(Val::Row(Word::Doc(c))),
                Val::Param(Param { key, doc: false }) => Some(Val::Param(Param { key, doc: true })),
                // A document's document: left to the step.
                _ => None,
            }
        };
        // A view's step a chunk reads at once (t57 E6, decision 0706): a test that binds nothing, or a lookup's
        // (`lookups`, not negated), its node a row's column.
        let lookup = lookups.get(at).copied().unwrap_or(false);
        let mut viewed = match step {
            Step::Atom { binds, neg, .. } if binds.is_empty() || (lookup && !*neg) => view_test(&mut b, &env, val, step, view),
            _ => None,
        };
        let mask: Option<Option<u16>> = match *step {
            Step::Unused { .. } => Some(None),
            Step::Lt { a, b: y, or_eq } => b.compare(val(&env, a), if or_eq { CmpOp::Le } else { CmpOp::Lt }, val(&env, y)).map(Some),
            Step::Eq { a, b: y } => b.compare(val(&env, a), CmpOp::Eq, val(&env, y)).map(Some),
            Step::Ne { a, b: y } => b.compare(val(&env, a), CmpOp::Ne, val(&env, y)).map(Some),
            Step::Doc { a, b: y, eq } => match (doc(val(&env, a)), doc(val(&env, y))) {
                (Some(x), Some(y)) => b.compare(x, if eq { CmpOp::Eq } else { CmpOp::Ne }, y).map(Some),
                _ => None,
            },
            Step::DocOf { out, n, bind } => match doc(val(&env, n)) {
                Some(d) if bind => {
                    env.push((out, d));
                    Some(None)
                }
                Some(d) => b.compare(val(&env, Key::Slot(out)), CmpOp::Eq, d).map(Some),
                None => None,
            },
            Step::Let { a, b: y } => {
                env.push((a, val(&env, y)));
                Some(None)
            }
            Step::In { a, ref set, want } if set.len() <= 64 => match val(&env, a) {
                Val::Row(w) => {
                    let col = b.column(w);
                    let mut any = b.op(BlockOp::Const(0));
                    for &c in set {
                        let rhs = Rhs::Param(b.param(Param { key: Key::Const(c), doc: false }));
                        let eq = b.op(BlockOp::Cmp { col, op: CmpOp::Eq, rhs, width: 64 });
                        any = b.op(BlockOp::Or(any, eq));
                    }
                    Some(Some(if want { any } else { b.op(BlockOp::Not(any)) }))
                }
                Val::Param(_) => None,
            },
            // A view's test (`viewed`): its verdicts a computed column, asked of the view a chunk at a time.
            Step::Atom { neg, ref keys, ref binds, .. } if viewed.is_some() => {
                // A node the view cannot read in a batch (a values part's) is asked as one row: its keys' slots.
                let mut slots = Vec::new();
                for &(_, k) in keys {
                    if let Key::Slot(s) = k {
                        match val(&env, k) {
                            Val::Row(w) => slots.push((s, Bind::Column(usize::from(b.column(w))))),
                            Val::Param(p) if p.key != Key::Slot(s) => slots.push((s, Bind::Param(usize::from(b.param(p))))),
                            Val::Param(_) => {}
                        }
                    }
                }
                tests.push(Test { at, neg, slots, view: viewed.take() });
                let t = tests.len() - 1;
                // A lookup's values: computed columns of the rows that pass.
                for &(c, s) in binds {
                    env.push((s, Val::Row(Word::Look(t, c))));
                    b.column(Word::Look(t, c));
                }
                let col = Val::Row(Word::Held(t));
                b.compare(col, CmpOp::Eq, Val::Param(Param { key: Key::Const(1), doc: false })).map(Some)
            }
            // A test of an input whose keys read the row (`Src::In`: an input, fixed while the rule runs; the
            // relations the stratum derives change under it): its verdicts a computed column.
            Step::Atom { src: super::Src::In(..), neg, ref keys, ref binds, .. } if ask && binds.is_empty() => {
                let mut slots = Vec::new();
                let mut row = false;
                let mut ok = true;
                for &(_, k) in keys {
                    if let Key::Slot(s) = k {
                        match val(&env, k) {
                            Val::Row(w @ Word::Col(_)) => {
                                row = true;
                                slots.push((s, Bind::Column(usize::from(b.column(w)))));
                            }
                            Val::Param(p @ Param { doc: false, .. }) => {
                                // The slot itself, bound before the atom, needs nothing; another (a `let`'s) is set.
                                if p.key != Key::Slot(s) {
                                    slots.push((s, Bind::Param(usize::from(b.param(p)))));
                                }
                            }
                            _ => ok = false,
                        }
                    }
                }
                if ok && row {
                    tests.push(Test { at, neg, slots, view: None });
                    let col = Val::Row(Word::Held(tests.len() - 1));
                    b.compare(col, CmpOp::Eq, Val::Param(Param { key: Key::Const(1), doc: false })).map(Some)
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(mask) = mask else { break };
        if let Some(m) = mask {
            steps = at + 1;
            acc = Some(match acc {
                Some(a) => b.op(BlockOp::And(a, m)),
                None => m,
            });
        }
        passed.push(acc);
    }
    if steps == 0 {
        return None;
    }
    // The steps after the last test are left to themselves.
    passed.truncate(steps);
    tests.retain(|t| t.at < steps);
    // The kernel's shape is a scan and the tests on what it binds: every slot the atom binds is read by a fused test.
    // An atom whose tests leave one of its slots unread (`p(c, o, m), f(k, m)`, `$c` the head's alone) is factored by
    // the walk instead (`setfix::factored`): its tests asked once per group of what they read, not once per row.
    if ask && !tests.is_empty() {
        let read = |s: usize| tests.iter().any(|t| {
            let Step::Atom { keys, .. } = &after[t.at] else { return false };
            keys.iter().any(|&(_, k)| k == Key::Slot(s))
        });
        if !binds.iter().all(|&(_, s)| read(s)) {
            return fused_viewing(binds, after, false, lookups, view);
        }
    }
    // Only the binds the kept steps made, and the atom's.
    let kept_binds = binds.len() + count_binds(&after[..steps]) + tests.iter().map(|t| t.view.as_ref().map_or(0, |v| v.looks.len())).sum::<usize>();
    env.truncate(kept_binds);
    // The outputs: each distinct running mask, the last one's rows extracted.
    let mut outputs: Vector<u16> = Vector::new();
    let index = |v: u16, outputs: &mut Vector<u16>| outputs.iter().position(|&o| o == v).unwrap_or_else(|| {
        outputs.push(v);
        outputs.len() - 1
    });
    let passed: Vec<Option<usize>> = passed.iter().map(|p| p.map(|v| index(v, &mut outputs))).collect();
    let last = passed.last().copied().flatten().expect("a test among the kept steps");
    let mut bind_list = Vec::new();
    for &(slot, v) in &env {
        bind_list.push((slot, match v {
            Val::Row(w) => Bind::Column(usize::from(b.column(w))),
            Val::Param(p) => Bind::Param(usize::from(b.param(p))),
        }));
    }
    // A column a computed one reads is gathered too.
    for k in 0..b.columns.len() {
        if let Word::Doc(c) = b.columns[k] {
            b.column(Word::Col(c));
        }
    }
    let extracts = rerail_blocks::vector![outputs[last]];
    let shape = RowShape { columns: b.columns.len() as u16, params: b.params.len() as u16, sets: 0 };
    let program = BlockProgram::row_program(shape, b.ops, outputs, extracts).ok()?;
    let gather = b.columns.iter().enumerate().filter_map(|(k, w)| match *w {
        Word::Col(c) => Some((k, c)),
        Word::Doc(_) | Word::Held(_) | Word::Look(..) => None,
    }).collect();
    Some(Fused { steps, program, columns: b.columns, params: b.params, passed, last, binds: bind_list, gather, tests })
}

/// A view's step a chunk reads at once (`Test::view`), as `ViewTest` says: a pair test, or a node's own row by its
/// id (a test of it, or a lookup's, its values `binds` read with it). `None` where the step is not one: its source no
/// view; no key a row's value the view reads its rows by their own id in (`Source::read_own_batch` asked with no
/// values); a key neither a row's value nor a parameter. A row's value is one of the atom's columns or a lookup's
/// before it (a computed column made before the tests), not a document (made after them). A node the view cannot read
/// in a batch (a values part's) is asked as one row.
fn view_test(b: &mut Build, env: &[(usize, Val)], val: impl Fn(&[(usize, Val)], Key) -> Val, step: &Step, view: ViewOf<'_>) -> Option<ViewTest> {
    let Step::Atom { src, keys, tests, binds, .. } = step else { return None };
    let (keys, tests, binds): (&[(usize, Key)], &[(usize, usize)], &[(usize, usize)]) = (keys, tests, binds);
    let (source, cols) = view(src)?;
    if keys.is_empty() {
        return None;
    }
    let at = |k: usize| cols.get(keys[k].0).copied();
    let row = |v: Val| matches!(v, Val::Row(Word::Col(_) | Word::Look(..)));
    let known = |v: Val| row(v) || matches!(v, Val::Param(Param { doc: false, .. }));
    let side = |b: &mut Build, v: Val| match v {
        Val::Row(w) => PairKey::Col(usize::from(b.column(w))),
        Val::Param(p) => PairKey::Param(usize::from(b.param(p))),
    };
    // A pair of the row's values, where the source tests pairs of those columns in batches.
    if let [(_, ka), (_, kb)] = keys
        && binds.is_empty()
        && tests.is_empty()
        && let (va, vb) = (val(env, *ka), val(env, *kb))
        && (row(va) || row(vb))
        && known(va)
        && known(vb)
        && let (Some(ca), Some(cb)) = (at(0), at(1))
        && super::kinds::keyed_enabled()
        && source.holds_batch([ca, cb], &[], &mut [])
    {
        let pair = [side(b, va), side(b, vb)];
        return Some(ViewTest { src: source, node: pair[0], by: ca, reads: Vec::new(), checks: Vec::new(), equal: Vec::new(), looks: Vec::new(), col: cb, pair: Some(pair) });
    }
    // A node's own row: the node's key, then every other column the step reads, once.
    let node = (0..keys.len()).find(|&k| row(val(env, keys[k].1)) && at(k).is_some_and(|c| source.read_own_batch(c, &[], &[], &mut [], &mut [], &mut ProbeCursor::default())))?;
    if !keys.iter().all(|&(_, k)| known(val(env, k))) {
        return None;
    }
    let by = at(node)?;
    let mut reads: Vec<usize> = Vec::new();
    let mut place = |c: usize| -> Option<usize> {
        let c = *cols.get(c)?;
        Some(reads.iter().position(|&r| r == c).unwrap_or_else(|| {
            reads.push(c);
            reads.len() - 1
        }))
    };
    let mut checks = Vec::new();
    for (k, &(c, key)) in keys.iter().enumerate() {
        if k != node {
            checks.push((place(c)?, key));
        }
    }
    let equal = tests.iter().map(|&(x, y)| Some((place(x)?, place(y)?))).collect::<Option<Vec<_>>>()?;
    let looks = binds.iter().map(|&(c, _)| Some((c, place(c)?))).collect::<Option<Vec<_>>>()?;
    let node = side(b, val(env, keys[node].1));
    let checks = checks.into_iter().map(|(r, key)| (r, side(b, val(env, key)))).collect();
    Some(ViewTest { src: source, node, by, reads, checks, equal, looks, col: by, pair: None })
}

/// The slots the steps bind (`docof` into an unbound slot, `let`), in order.
fn count_binds(steps: &[Step]) -> usize {
    steps.iter().filter(|s| matches!(s, Step::DocOf { bind: true, .. } | Step::Let { .. })).count()
}

/// A chunk's columns and the program's values, kept from chunk to chunk.
#[derive(Default)]
pub(super) struct Scratch {
    pub(super) columns: Vec<Vector<i64>>,
    pub(super) params: Vector<i64>,
    pub(super) values: Vector<u64>,
    pub(super) rows: usize,
    /// The chunk's rows' ids in their relation, when the fused atom is the join's first step (`table`'s counts per row).
    pub(super) ids: Vector<u32>,
    /// The rows still passing the fused tests (`Fused::run`), a bit each.
    pub(super) alive: Vector<u64>,
    /// Each fused test's probe state, kept from chunk to chunk.
    pub(super) asked: Vec<super::cands::Asked>,
    /// Each fused test's view cursor (`Test::view`), kept from chunk to chunk: a chunk's nodes after the last's.
    pub(super) cursors: Vec<ProbeCursor>,
    /// A view's test's rows still passing, their nodes, the tested column's values and whether each has a row
    /// (`Source::read_own_batch`).
    pub(super) view_rows: Vector<u32>,
    pub(super) view_ids: Vector<i64>,
    pub(super) view_out: Vector<i64>,
    pub(super) view_found: Vector<u8>,
}

impl Fused {
    /// The chunk's computed columns from its gathered ones, then the program over the chunk:
    /// calls `each` with each fused step's passed count (`counts`), then returns the output
    /// words of the rows that go on. `held(test, row, columns, params, out)` is whether test `test`'s input (`Fused::tests`) holds row `row`'s keys,
    /// asked only of rows that passed the tests before it; a lookup's (`ViewTest::looks`, a node its view cannot read in a batch) writes the values of its
    /// input's columns to `out` (by column), which become its `Word::Look` columns.
    pub(super) fn run<'s>(&'s self, s: &'s mut Scratch, mut each: impl FnMut(usize, u64), mut held: impl FnMut(usize, usize, &[Vector<i64>], &[i64], &mut [i64]) -> bool) -> &'s [u64] {
        let n = s.rows;
        if !self.tests.is_empty() {
            // The rows still passing, a bit each: a test is asked only of rows every test before it passed.
            let mut alive = std::mem::take(&mut s.alive);
            alive.clear();
            alive.resize(n.div_ceil(64), u64::MAX);
            s.cursors.resize_with(self.tests.len(), ProbeCursor::default);
            for (t, test) in self.tests.iter().enumerate() {
                let k = self.columns.iter().position(|&w| w == Word::Held(t)).expect("a test's column");
                let mut out = std::mem::take(&mut s.columns[k]);
                out.clear();
                out.resize(n, 0);
                if let Some(v) = &test.view
                    && let Some(pair) = v.pair
                {
                    let side = |k: PairKey, r: usize| match k {
                        PairKey::Col(c) => s.columns[c][r],
                        PairKey::Param(p) => s.params[p],
                    };
                    // The rows still passing, their pairs asked of the source as one batch (decision 0706 E5).
                    let (mut rows, mut ids, mut found) = (std::mem::take(&mut s.view_rows), std::mem::take(&mut s.view_ids), std::mem::take(&mut s.view_found));
                    rows.clear();
                    ids.clear();
                    for r in 0..n {
                        if alive[r / 64] >> (r % 64) & 1 != 0 {
                            rows.push(r as u32);
                            ids.extend([side(pair[0], r), side(pair[1], r)]);
                        }
                    }
                    found.clear();
                    found.resize(rows.len(), 0);
                    super::kinds::count_pairs_for_tests(rows.len());
                    let answered = v.src.holds_batch([v.by, v.col], &ids, &mut found);
                    assert!(answered, "a view testing pairs of these columns tests them in a batch");
                    for (j, &r) in rows.iter().enumerate() {
                        let r = r as usize;
                        let h = match found[j] {
                            0 => false,
                            1 => true,
                            // A pair the source cannot say in a batch: asked as one row.
                            _ => held(t, r, &s.columns, &s.params, &mut [0; super::READ]),
                        };
                        if h != test.neg { out[r] = 1; } else { alive[r / 64] &= !(1u64 << (r % 64)); }
                    }
                    (s.view_rows, s.view_ids, s.view_found) = (rows, ids, found);
                } else if let Some(v) = &test.view {
                    // The rows still passing, their nodes' rows read by the view as one batch (t57 E6, decision 0706
                    // E1): each checked as the step's keys and tests say, and a lookup's values its columns.
                    let side = |k: PairKey, r: usize, columns: &[Vector<i64>]| match k {
                        PairKey::Col(c) => columns[c][r],
                        PairKey::Param(p) => s.params[p],
                    };
                    let (mut rows, mut ids, mut got, mut found) = (std::mem::take(&mut s.view_rows), std::mem::take(&mut s.view_ids), std::mem::take(&mut s.view_out), std::mem::take(&mut s.view_found));
                    rows.clear();
                    ids.clear();
                    for r in 0..n {
                        if alive[r / 64] >> (r % 64) & 1 != 0 {
                            rows.push(r as u32);
                            ids.push(side(v.node, r, &s.columns));
                        }
                    }
                    let w = v.reads.len();
                    got.clear();
                    got.resize(ids.len() * w, 0);
                    found.clear();
                    found.resize(ids.len(), 0);
                    // Fused only where the source reads rows by their own id (`view_test`).
                    let answered = v.src.read_own_batch(v.by, &ids, &v.reads, &mut got, &mut found, &mut s.cursors[t]);
                    assert!(answered, "a view keyed by its node's column reads its rows in a batch");
                    // A lookup's columns, out while they are written: (program column, its read, the atom's column).
                    let mut looks: Vec<(usize, usize, usize, Vector<i64>)> = v.looks.iter().map(|&(c, at)| {
                        let k = self.columns.iter().position(|&x| x == Word::Look(t, c)).expect("a lookup's column");
                        let mut col = std::mem::take(&mut s.columns[k]);
                        col.clear();
                        col.resize(n, 0);
                        (k, at, c, col)
                    }).collect();
                    let mut one = [0i64; super::READ];
                    for (j, &r) in rows.iter().enumerate() {
                        let r = r as usize;
                        let row = &got[j * w..(j + 1) * w];
                        let h = match found[j] {
                            0 => false,
                            1 => v.checks.iter().all(|&(at, key)| row[at] == side(key, r, &s.columns)) && v.equal.iter().all(|&(a, c)| row[a] == row[c]),
                            // A node the view cannot read in a batch: asked as one row, its values by the atom's column.
                            _ => {
                                let h = held(t, r, &s.columns, &s.params, &mut one);
                                if h {
                                    for (_, _, c, col) in &mut looks { col[r] = one[*c]; }
                                }
                                if h != test.neg { out[r] = 1; } else { alive[r / 64] &= !(1u64 << (r % 64)); }
                                continue;
                            }
                        };
                        if h {
                            for (_, at, _, col) in &mut looks { col[r] = row[*at]; }
                        }
                        if h != test.neg { out[r] = 1; } else { alive[r / 64] &= !(1u64 << (r % 64)); }
                    }
                    for (k, _, _, col) in looks {
                        s.columns[k] = col;
                    }
                    (s.view_rows, s.view_ids, s.view_out, s.view_found) = (rows, ids, got, found);
                } else {
                    for (r, o) in out.iter_mut().enumerate() {
                        if alive[r / 64] >> (r % 64) & 1 == 0 { continue; }
                        if held(t, r, &s.columns, &s.params, &mut [0; super::READ]) != test.neg { *o = 1; } else { alive[r / 64] &= !(1u64 << (r % 64)); }
                    }
                }
                s.columns[k] = out;
            }
            s.alive = alive;
        }
        for (k, w) in self.columns.iter().enumerate() {
            if let Word::Doc(c) = *w {
                let from = self.columns.iter().position(|&x| x == Word::Col(c)).expect("gathered");
                let (src, dst) = if from < k {
                    let (a, b) = s.columns.split_at_mut(k);
                    (&a[from], &mut b[0])
                } else {
                    let (a, b) = s.columns.split_at_mut(from);
                    (&b[0], &mut a[k])
                };
                dst.clear();
                dst.extend(src.iter().map(|&x| rerail_blocks::located::Located::key_wide(x)));
            }
        }
        // On the stack (decision 0687): a fused chunk's columns and outputs are few.
        let cols: InlineVec<&[i64], 8> = s.columns.iter().map(|v| v.as_slice()).collect();
        let batch = RowBatch { columns: &cols, params: &s.params, sets: &[], len: n };
        s.values.resize(self.program.row_words(n), 0);
        let outputs: InlineVec<&'s [u64], 8> = self.program.interpret_rows(&batch, &mut s.values[..]).collect();
        for (m, p) in self.passed.iter().enumerate() {
            each(m, p.map_or(n as u64, |k| outputs[k].iter().map(|w| u64::from(w.count_ones())).sum()));
        }
        outputs[self.last]
    }
}
