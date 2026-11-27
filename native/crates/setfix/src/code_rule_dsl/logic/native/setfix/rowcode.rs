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
//!   asked one input at a time, not a row at a time through the walk.
//!
//! The steps' observed fanouts (`JoinRule::counts`) stay exact: the program outputs each fused
//! step's running mask, and its popcount is the rows that step passed on.

use rerail_blocks::frame::InlineVec;
use rerail_blocks::vector::Vector;
use super::{Key, ScalarKind, Step};
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
    /// Lookup test `k`'s (`Test::reads`) value of its input's column `c` for the row: the one row its key names, read
    /// as the test asks it.
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
    /// A lookup's (`setfix::lookups`): the input's columns it binds, each a computed column (`Word::Look`) of the
    /// row its key names. Empty for a test that binds nothing.
    pub reads: Vec<usize>,
}

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
/// A lookup (`lookups`, `setfix::lookups`) is no atom's scan: its one row is read in place, by the walk or, after a
/// fused atom, as the chunk's computed columns.
pub(super) fn fuse(steps: &[Step], lookups: &[bool]) -> Vec<Option<Box<Fused>>> {
    let mut out: Vec<Option<Box<Fused>>> = (0..steps.len()).map(|_| None).collect();
    for (i, step) in steps.iter().enumerate() {
        if steps[..i].iter().any(|s| matches!(s, Step::Scalar { kind: ScalarKind::Sum, bind: true, .. })) {
            break;
        }
        if let Step::Atom { neg: false, binds, .. } = step
            && !binds.is_empty()
            && !lookups.get(i).copied().unwrap_or(false)
        {
            out[i] = fused(binds, &steps[i + 1..], tests_on(), lookups.get(i + 1..).unwrap_or(&[])).map(Box::new);
        }
    }
    out
}

/// Whether a test of an input is fused into its scan's chunk (`Test`): `RERAIL_FUSED_TESTS=off` (a measurement aid,
/// the run's setting) leaves it to the walk, as before.
fn tests_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_FUSED_TESTS").map_or(true, |v| v != "off"))
}

/// `ask`: whether a test of an input is fused (`Test`), which only the CPU's walk asks; `lookups`, per step of
/// `after`, whether it is a lookup (`fuse`), fused as computed columns where `ask`.
pub fn fused(binds: &[(usize, usize)], after: &[Step], ask: bool, lookups: &[bool]) -> Option<Fused> {
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
            // A lookup whose key reads the row: the one row it names read for each row of the chunk, its verdict and
            // the values it binds computed columns (`Word::Held`, `Word::Look`), no probe's loop.
            Step::Atom { src: super::Src::In(..), neg, ref keys, ref binds, .. } if ask && lookups.get(at).copied().unwrap_or(false) && (binds.is_empty() || !neg) => {
                let mut slots = Vec::new();
                let mut row = false;
                let mut ok = true;
                for &(_, k) in keys {
                    if let Key::Slot(s) = k {
                        match val(&env, k) {
                            // A computed column is made before the tests (a test's, by the tests before it), but a
                            // document's after them: not read by one.
                            Val::Row(w @ (Word::Col(_) | Word::Look(..))) => {
                                row = true;
                                slots.push((s, Bind::Column(usize::from(b.column(w)))));
                            }
                            Val::Param(p @ Param { doc: false, .. }) => {
                                if p.key != Key::Slot(s) {
                                    slots.push((s, Bind::Param(usize::from(b.param(p)))));
                                }
                            }
                            _ => ok = false,
                        }
                    }
                }
                if ok && row {
                    tests.push(Test { at, neg, slots, reads: binds.iter().map(|&(c, _)| c).collect() });
                    let t = tests.len() - 1;
                    for &(c, s) in binds {
                        env.push((s, Val::Row(Word::Look(t, c))));
                    }
                    let held = Val::Row(Word::Held(t));
                    b.compare(held, CmpOp::Eq, Val::Param(Param { key: Key::Const(1), doc: false })).map(Some)
                } else {
                    None
                }
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
                    tests.push(Test { at, neg, slots, reads: Vec::new() });
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
    if !tests.is_empty() {
        let read = |s: usize| tests.iter().any(|t| {
            let Step::Atom { keys, .. } = &after[t.at] else { return false };
            keys.iter().any(|&(_, k)| k == Key::Slot(s))
        });
        if !binds.iter().all(|&(_, s)| read(s)) {
            return fused(binds, after, false, lookups);
        }
    }
    // Only the binds the kept steps made, and the atom's.
    let kept_binds = binds.len() + count_binds(&after[..steps]) + tests.iter().map(|t| t.reads.len()).sum::<usize>();
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
}

impl Fused {
    /// The chunk's computed columns from its gathered ones, then the program over the chunk:
    /// calls `each` with each fused step's passed count (`counts`), then returns the output
    /// words of the rows that go on. `ask(test, columns, params, alive, rows, verdict, looks)` asks test `test`'s input
    /// (`Fused::tests`) of the chunk's rows at once, those still `alive` (a bit a row) alone: for each that passes it
    /// writes 1 to column `verdict`, and a lookup's (`Test::reads`) values of its input's columns to `looks`' program
    /// columns (program column, input column), its `Word::Look` columns; each that fails it clears in `alive`.
    pub(super) fn run<'s>(&'s self, s: &'s mut Scratch, mut each: impl FnMut(usize, u64), mut ask: impl FnMut(usize, &mut [Vector<i64>], &[i64], &mut [u64], usize, usize, &[(usize, usize)])) -> &'s [u64] {
        let n = s.rows;
        if !self.tests.is_empty() {
            // The rows still passing, a bit each: a test is asked only of rows every test before it passed.
            let mut alive = std::mem::take(&mut s.alive);
            alive.clear();
            alive.resize(n.div_ceil(64), u64::MAX);
            for t in 0..self.tests.len() {
                let k = self.columns.iter().position(|&w| w == Word::Held(t)).expect("a test's column");
                s.columns[k].clear();
                s.columns[k].resize(n, 0);
                // A lookup's columns: (program column, input column).
                let looks: InlineVec<(usize, usize), 8> = self.columns.iter().enumerate().filter_map(|(k, &w)| match w { Word::Look(u, c) if u == t => Some((k, c)), _ => None }).collect();
                for &(k, _) in looks.iter() {
                    s.columns[k].clear();
                    s.columns[k].resize(n, 0);
                }
                ask(t, &mut s.columns, &s.params, &mut alive, n, k, &looks);
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
                dst.extend(src.iter().map(|&x| x >> 32));
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
