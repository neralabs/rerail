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
//! The steps' observed fanouts (`JoinRule::counts`) stay exact: the program outputs each fused
//! step's running mask, and its popcount is the rows that step passed on.

use super::{Key, ScalarKind, Step};
use crate::code_rule_dsl::transducer::packed::block::{BlockOp, BlockProgram, CmpOp, Rhs, RowBatch, RowShape};

/// The rows gathered before the program runs over them.
pub(super) const CHUNK: usize = 1024;

/// A value of the chunk's rows: an atom column, or a value computed from one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Word {
    /// The atom's column.
    Col(usize),
    /// The atom's column's document: `@doc(x)`, `x >> 32`.
    Doc(usize),
}

/// A value the same for every row of a chunk: a slot bound before the atom, or a constant, or
/// its document.
#[derive(Clone, Copy)]
pub(super) struct Param {
    pub(super) key: Key,
    pub(super) doc: bool,
}

/// A slot's value within the fused steps.
#[derive(Clone, Copy)]
enum Val {
    Row(Word),
    Param(Param),
}

/// An atom fused with the steps after it.
pub(super) struct Fused {
    /// The steps fused after the atom.
    pub(super) steps: usize,
    pub(super) program: BlockProgram,
    /// The program's columns, each a `Word` of the atom's rows; the raw ones first are gathered,
    /// the computed ones are evaluated from them.
    pub(super) columns: Vec<Word>,
    pub(super) params: Vec<Param>,
    /// For each fused step, the program output whose rows it passed on (`None`: every row).
    pub(super) passed: Vec<Option<usize>>,
    /// The output whose rows go on.
    pub(super) last: usize,
    /// The slots set for each row that goes on, from the program's columns or parameters.
    pub(super) binds: Vec<(usize, Bind)>,
    /// The gathered columns: (program column, atom column).
    pub(super) gather: Vec<(usize, usize)>,
}

#[derive(Clone, Copy)]
pub(super) enum Bind {
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
pub(super) fn fuse(steps: &[Step]) -> Vec<Option<Box<Fused>>> {
    let mut out: Vec<Option<Box<Fused>>> = (0..steps.len()).map(|_| None).collect();
    for (i, step) in steps.iter().enumerate() {
        if steps[..i].iter().any(|s| matches!(s, Step::Scalar { kind: ScalarKind::Sum, bind: true, .. })) {
            break;
        }
        if let Step::Atom { neg: false, binds, .. } = step
            && !binds.is_empty()
        {
            out[i] = fused(binds, &steps[i + 1..]).map(Box::new);
        }
    }
    out
}

pub(super) fn fused(binds: &[(usize, usize)], after: &[Step]) -> Option<Fused> {
    let mut env: Vec<(usize, Val)> = binds.iter().map(|&(c, s)| (s, Val::Row(Word::Col(c)))).collect();
    let mut b = Build::default();
    let mut acc: Option<u16> = None;
    let mut passed: Vec<Option<u16>> = Vec::new();
    let mut steps = 0;
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
    // Only the binds the kept steps made, and the atom's.
    let kept_binds = binds.len() + count_binds(&after[..steps]);
    env.truncate(kept_binds);
    // The outputs: each distinct running mask, the last one's rows extracted.
    let mut outputs: Vec<u16> = Vec::new();
    let mut index = |v: u16, outputs: &mut Vec<u16>| outputs.iter().position(|&o| o == v).unwrap_or_else(|| {
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
    let extracts = vec![outputs[last]];
    let shape = RowShape { columns: b.columns.len() as u16, params: b.params.len() as u16, sets: 0 };
    let program = BlockProgram::row_program(shape, b.ops, outputs, extracts).ok()?;
    let gather = b.columns.iter().enumerate().filter_map(|(k, w)| match *w {
        Word::Col(c) => Some((k, c)),
        Word::Doc(_) => None,
    }).collect();
    Some(Fused { steps, program, columns: b.columns, params: b.params, passed, last, binds: bind_list, gather })
}

/// The slots the steps bind (`docof` into an unbound slot, `let`), in order.
fn count_binds(steps: &[Step]) -> usize {
    steps.iter().filter(|s| matches!(s, Step::DocOf { bind: true, .. } | Step::Let { .. })).count()
}

/// A chunk's columns and the program's values, kept from chunk to chunk.
#[derive(Default)]
pub(super) struct Scratch {
    pub(super) columns: Vec<Vec<i64>>,
    pub(super) params: Vec<i64>,
    pub(super) values: Vec<u64>,
    pub(super) rows: usize,
    /// The chunk's rows' ids in their relation, when the fused atom is the join's first step (`table`'s counts per row).
    pub(super) ids: Vec<u32>,
}

impl Fused {
    /// The chunk's computed columns from its gathered ones, then the program over the chunk:
    /// calls `each` with each fused step's passed count (`counts`), then returns the output
    /// words of the rows that go on.
    pub(super) fn run<'s>(&'s self, s: &'s mut Scratch, mut each: impl FnMut(usize, u64)) -> &'s [u64] {
        let n = s.rows;
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
        let cols: Vec<&[i64]> = s.columns.iter().map(Vec::as_slice).collect();
        let batch = RowBatch { columns: &cols, params: &s.params, sets: &[], len: n };
        let outputs: Vec<&'s [u64]> = self.program.interpret_rows(&batch, &mut s.values).collect();
        for (m, p) in self.passed.iter().enumerate() {
            each(m, p.map_or(n as u64, |k| outputs[k].iter().map(|w| u64::from(w.count_ones())).sum()));
        }
        outputs[self.last]
    }
}
