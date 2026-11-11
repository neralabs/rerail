//! A stratum's joins on the device (decision 0095, step 4): the native join plan run by Metal, when the placement
//! planner puts the stratum there (`plan::placement`, under the Metal binding).
//!
//! **What runs where.** The planner writes the stratum's spec as for the CPU, with one more line, `device`. At
//! `open`, each of its seed joins is compiled into device stages (`compile`). A stratum the device cannot run is
//! refused whole, with the step that stopped it (`refused`): it then runs on the CPU, and the profile names the
//! reason. Nothing is rerouted silently. The device takes:
//!
//! - an atom over an input relation or a lower stratum's relation, read by its bound keys (or whole), with its
//!   column tests: a join stage;
//! - a negated atom, or an atom that binds nothing: an anti- or semi-join stage, by the same keys;
//! - the filters fused after an atom (`rowcode`: `lt`, `le`, `eq`, `ne`, `in`, `deq`, `dne`, `docof`, `let`): the
//!   CPU's own row program, run in the join stage per candidate; filters after a step that binds nothing run as a
//!   filter stage over the bindings, by the same program form;
//! - `let` and `docof` that bind a slot: a map, in place;
//! - a head of slots and constants.
//!
//! It does not take string joins, text tests, computed text (`cat`, text functions), gates or requests: the forms
//! (recursion), gates or requests: the forms `DEVICE_FORMS` lacks.
//!
//! **One submission per stratum.** Every seed join of the stratum is encoded into one `metal_join::Batch`, and the
//! relations they read stay on the device across the chunk's strata (`Resident`): uploaded once, sorted once per key,
//! while the engine holds the relation (a weak reference: a relation the engine drops is dropped here too). A stage
//! whose rows overflow the space its table was given is run again from that stage with the exact size, in one more
//! submission for every rule that overflowed.
//!
//! **The same result.** The head rows go into the store exactly as the CPU's do (a row already there, or found twice,
//! is dropped by the store's own set), and each step's observed counts (the bindings that entered it and that it
//! passed on) are the CPU's, so the replanner sees the same fanouts.

use super::{InRel, Interner, JoinRule, Key, Src, Step, Store, Term, Fresh, is_dead, rowcode, thread_cpu};
use crate::code_rule_dsl::transducer::packed::metal_join as mj;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::time::Instant;

/// The most rows a table is given before its stage has run once (a guess; an overflow runs it again exactly).
const GUESS_MAX: usize = 1 << 21;
/// The device's buffers share one budget of 256 MB in the process (`metal_backend`'s ledger), with the buffer pool's
/// 64 MB and the other lanes. The device joins keep within about half of it:
/// the most bytes one binding table may take: past it the rule is refused at run time and runs on the CPU, named;
const TABLE_MAX: usize = 48 << 20;
/// the most bytes one submission's tables may take: past it the rules after go into the next submission;
const BATCH_MAX: usize = 64 << 20;
/// the most bytes the relations kept on the device may take on a thread;
const RESIDENT_MAX: usize = 80 << 20;
/// the budget a submission leaves for its stages run again after an overflow: with less left, the rules after go
/// into the next submission.
const HEADROOM: usize = 48 << 20;

/// How a stage reads.
#[derive(Clone)]
enum Kind {
    /// A step that does nothing (`Unused`).
    Pass,
    /// Slots set in place.
    Map,
    /// The row program over the bindings.
    Filter,
    /// An atom: its relation and key columns, in key order.
    Atom { rel: Arc<InRel>, cols: Vec<usize> },
    /// An atom over the component's own relation `rel` (recursion, `fixpoint`): its last round's new rows
    /// (`delta`) or all of them, kept on the device by the rounds (`Own`).
    Own { rel: usize, delta: bool, cols: Vec<usize> },
    /// A text test of a slot bound from column `col` of `rel`: the verdicts of `table` (keyed `key`) for the
    /// column's distinct symbols, made on the device and kept with the relation.
    Text { rel: Arc<InRel>, col: usize, table: Arc<crate::code_rule_dsl::text::table::Table>, key: String },
    /// `str_cmp`: the texts of the columns that bound its slots and of its constants, on the device together.
    Strcmp { cols: Vec<(Arc<InRel>, usize)>, consts: Vec<i64> },
}

#[derive(Clone)]
struct PStage {
    /// The first step it covers, and how many.
    first: usize,
    span: usize,
    kind: Kind,
    desc: mj::Stage,
    /// Fused steps whose passes the stage counts (`Stage::npassed`).
    npassed: usize,
}

/// A join rule's device plan.
#[derive(Clone)]
pub(super) struct Plan {
    stages: Vec<PStage>,
    head: Vec<Option<i64>>,
    head_slots: Vec<usize>,
    nslots: usize,
    nsteps: usize,
    /// The rule's spec, hashed: its stages' observed fanouts are kept by it (`GROWTH`), for the next chunk's guesses.
    key: u64,
}

fn step_name(s: &Step) -> &'static str {
    match s {
        Step::Unused { .. } => "unused",
        Step::Atom { .. } => "atom",
        Step::Cat { .. } => "cat",
        Step::Split { .. } => "split",
        Step::Eq { .. } => "eq",
        Step::Lt { .. } => "lt",
        Step::Scalar { .. } => "aggregate",
        Step::Let { .. } => "let",
        Step::Ne { .. } => "ne",
        Step::Func { .. } => "function",
        Step::Doc { .. } => "document test",
        Step::DocOf { .. } => "docof",
        Step::Test { .. } => "text test",
        Step::In { .. } => "in",
        Step::StrJoin { .. } => "string join",
    }
}

/// The device's operation for an integer function the native engine computes (`strings::functions`' names).
pub(in crate::code_rule_dsl) fn arith_op(name: &str) -> Option<u32> {
    Some(match name {
        "rr_add" => mj::ARITH_ADD,
        "rr_sub" => mj::ARITH_SUB,
        "rr_mul" => mj::ARITH_MUL,
        "rr_div" => mj::ARITH_DIV,
        "mod" => mj::ARITH_MOD,
        "rr_max" | "max" => mj::ARITH_MAX,
        "rr_min" | "min" => mj::ARITH_MIN,
        _ => return None,
    })
}

/// The program's columns, parameters, operations and outputs of a fused row program, into `d`; the source of a
/// column (`Word::Col(c)`) is `kind` (a candidate's column, or the binding's slot `c`).
fn program(d: &mut mj::Stage, f: &rowcode::Fused, kind: u32) -> Result<(), String> {
    use crate::code_rule_dsl::transducer::packed::block::{BlockOp, CmpOp, Rhs};
    if f.columns.len() > mj::MAX_COLS || f.params.len() > mj::MAX_PARAMS || f.program.ops().len() > mj::MAX_OPS || f.program.outputs().len() > mj::MAX_OUTPUTS || f.steps > mj::MAX_OUTPUTS || f.binds.len() > mj::MAX_BINDS {
        return Err("a row program larger than a stage holds".into());
    }
    for (k, w) in f.columns.iter().enumerate() {
        let (c, doc) = match *w { rowcode::Word::Col(c) => (c, 0), rowcode::Word::Doc(c) => (c, 1) };
        d.col_kind[k] = kind;
        d.col_idx[k] = c as u32;
        d.col_doc[k] = doc;
    }
    d.ncols = f.columns.len() as u32;
    for (k, p) in f.params.iter().enumerate() {
        match p.key {
            Key::Slot(s) => { d.par_kind[k] = mj::SLOT; d.par_slot[k] = s as u32; }
            Key::Const(c) => { d.par_kind[k] = mj::CONST; d.par_const[k] = c; }
        }
        d.par_doc[k] = u32::from(p.doc);
    }
    d.nparams = f.params.len() as u32;
    for (x, op) in f.program.ops().iter().enumerate() {
        d.ops[x] = match *op {
            BlockOp::Cmp { col, op, rhs, width } => {
                let b = match rhs { Rhs::Col(c) => u32::from(c), Rhs::Param(p) => mj::RHS_PARAM | u32::from(p) };
                let o = match op { CmpOp::Eq => 0, CmpOp::Ne => 1, CmpOp::Lt => 2, CmpOp::Le => 3 };
                [mj::OP_CMP, u32::from(col), b, o | (u32::from(width) << 8)]
            }
            BlockOp::Const(0) => [mj::OP_CONST, 0, 0, 0],
            BlockOp::Const(u64::MAX) => [mj::OP_CONST, 1, 0, 0],
            BlockOp::Not(a) => [mj::OP_NOT, u32::from(a), 0, 0],
            BlockOp::And(a, b) => [mj::OP_AND, u32::from(a), u32::from(b), 0],
            BlockOp::Or(a, b) => [mj::OP_OR, u32::from(a), u32::from(b), 0],
            BlockOp::Xor(a, b) => [mj::OP_XOR, u32::from(a), u32::from(b), 0],
            BlockOp::AndNot(a, b) => [mj::OP_ANDNOT, u32::from(a), u32::from(b), 0],
            other => return Err(format!("a row program's {other:?}")),
        };
    }
    d.nops = f.program.ops().len() as u32;
    for (k, &o) in f.program.outputs().iter().enumerate() {
        d.outputs[k] = u32::from(o);
    }
    d.noutputs = f.program.outputs().len() as u32;
    for (m, p) in f.passed.iter().enumerate() {
        d.passed[m] = p.map_or(mj::NONE, |k| k as u32);
    }
    d.npassed = f.steps as u32;
    d.last = f.last as u32;
    Ok(())
}

/// The binds of a fused program, into `d` (an identity bind of a slot to itself, a filter's, is left out).
fn binds(d: &mut mj::Stage, f: &rowcode::Fused, filter: bool) {
    let mut n = 0;
    for &(slot, b) in &f.binds {
        let (kind, idx) = match b { rowcode::Bind::Column(k) => (mj::BIND_COLUMN, k), rowcode::Bind::Param(k) => (mj::BIND_PARAM, k) };
        if filter && kind == mj::BIND_COLUMN && f.columns[idx] == rowcode::Word::Col(slot) {
            continue;
        }
        d.bind_slot[n] = slot as u32;
        d.bind_kind[n] = kind;
        d.bind_idx[n] = idx as u32;
        n += 1;
    }
    d.nbinds = n as u32;
}

/// Whether the device can read `rel` whole for an atom with `keys` bound: a source that makes its rows on lookup (an
/// axis, `within`) is read by the CPU a key at a time, and the device's upload would make every row of it. Read
/// whole (no keys), the CPU makes them all too, and the device takes it.
fn readable(rel: &InRel, keys: usize) -> Result<(), String> {
    match &rel.view {
        Some(v) if keys > 0 && v.src.made_on_lookup() => Err("an atom read by keys over a relation made on lookup".into()),
        _ => Ok(()),
    }
}

/// `j`'s device plan, or the step the device cannot run.
pub(super) fn compile(j: &JoinRule, spec: &str) -> Result<Plan, String> {
    let mut bound = vec![false; j.nslots];
    // Where each slot's value came from: a column of an input relation (a text test's terms are its symbols).
    let mut origin: Vec<Option<(Arc<InRel>, usize)>> = vec![None; j.nslots];
    let mut stages = Vec::new();
    let mut i = 0;
    while i < j.steps.len() {
        let step = &j.steps[i];
        match step {
            Step::Unused { .. } => {
                stages.push(PStage { first: i, span: 1, kind: Kind::Pass, desc: mj::Stage::default(), npassed: 0 });
                i += 1;
            }
            Step::Atom { src, neg, keys, binds: atom_binds, tests } => {
                let own = match src {
                    Src::In(rel, _) => {
                        readable(rel, keys.len())?;
                        None
                    }
                    Src::Delta(r) => Some((*r, true)),
                    Src::Full(r) => Some((*r, false)),
                    Src::FullMulti(..) => return Err("an atom over the component's own relation by several indexes".into()),
                };
                if keys.len() > mj::MAX_KEYS || tests.len() > mj::MAX_TESTS || atom_binds.len() > mj::MAX_BINDS {
                    return Err("an atom with more keys, tests or binds than a stage holds".into());
                }
                let mut d = mj::Stage { mode: if *neg { mj::ANTI } else if atom_binds.is_empty() { mj::SEMI } else { mj::JOIN }, ..mj::Stage::default() };
                for (k, &(c, key)) in keys.iter().enumerate() {
                    d.key_rcol[k] = c as u32;
                    match key {
                        Key::Slot(s) => { d.key_kind[k] = mj::SLOT; d.key_slot[k] = s as u32; }
                        Key::Const(v) => { d.key_kind[k] = mj::CONST; d.key_const[k] = v; }
                    }
                }
                d.nkeys = keys.len() as u32;
                for (k, &(a, b)) in tests.iter().enumerate() {
                    d.test_a[k] = a as u32;
                    d.test_b[k] = b as u32;
                }
                d.ntests = tests.len() as u32;
                let mut span = 1;
                let mut npassed = 0;
                match j.fused[i].as_deref() {
                    Some(f) if d.mode == mj::JOIN => {
                        program(&mut d, f, mj::RIGHT)?;
                        binds(&mut d, f, false);
                        for &(s, _) in &f.binds { bound[s] = true; }
                        span += f.steps;
                        npassed = f.steps;
                    }
                    _ => {
                        for (k, &(c, s)) in atom_binds.iter().enumerate() {
                            d.bind_slot[k] = s as u32;
                            d.bind_kind[k] = mj::BIND_RIGHT;
                            d.bind_idx[k] = c as u32;
                            bound[s] = true;
                        }
                        d.nbinds = atom_binds.len() as u32;
                    }
                }
                if !*neg {
                    for &(c, s) in atom_binds.iter() {
                        if s >= origin.len() {
                            origin.resize(s + 1, None);
                        }
                        origin[s] = match src { Src::In(rel, _) => Some((rel.clone(), c)), _ => None };
                    }
                }
                let cols = keys.iter().map(|&(c, _)| c).collect();
                let kind = match (own, src) {
                    (Some((rel, delta)), _) => Kind::Own { rel, delta, cols },
                    (None, Src::In(rel, _)) => Kind::Atom { rel: rel.clone(), cols },
                    (None, _) => unreachable!("an own source has its relation"),
                };
                stages.push(PStage { first: i, span, kind, desc: d, npassed });
                i += span;
            }
            Step::Scalar { src, keys, tests, col, kind, out, bind, .. } => {
                // An aggregate: each binding's matching rows folded where they lie, in the relation's sort by the
                // keys (one row out per binding with a value). A sum past 64 bits is refused at run time.
                use crate::code_rule_dsl::logic::analysis::scalars::Kind as K;
                let fold = match kind {
                    K::Count => mj::AGG_COUNT,
                    K::Sum => mj::AGG_SUM,
                    K::Min => mj::AGG_MIN,
                    K::Max => mj::AGG_MAX,
                    #[allow(unreachable_patterns)]
                    _ => return Err("an aggregate the device does not fold".into()),
                };
                if keys.len() > mj::MAX_KEYS || tests.len() > mj::MAX_TESTS {
                    return Err("an aggregate with more keys or tests than a stage holds".into());
                }
                let mut d = mj::Stage { mode: mj::AGG, ..mj::Stage::default() };
                for (k, &(c, key)) in keys.iter().enumerate() {
                    d.key_rcol[k] = c as u32;
                    match key {
                        Key::Slot(s) => { d.key_kind[k] = mj::SLOT; d.key_slot[k] = s as u32; }
                        Key::Const(v) => { d.key_kind[k] = mj::CONST; d.key_const[k] = v; }
                    }
                }
                d.nkeys = keys.len() as u32;
                for (k, &(a, b)) in tests.iter().enumerate() {
                    d.test_a[k] = a as u32;
                    d.test_b[k] = b as u32;
                }
                d.ntests = tests.len() as u32;
                d.agg = [fold | (u32::from(*bind) << 8), col.map_or(mj::NONE, |c| c as u32)];
                d.bind_slot[0] = *out as u32;
                if *bind {
                    if *out >= bound.len() {
                        bound.resize(*out + 1, false);
                    }
                    bound[*out] = true;
                }
                let cols = keys.iter().map(|&(c, _)| c).collect();
                let kind = match src {
                    Src::In(rel, _) => {
                        readable(rel, keys.len())?;
                        Kind::Atom { rel: rel.clone(), cols }
                    }
                    Src::Delta(r) => Kind::Own { rel: *r, delta: true, cols },
                    Src::Full(r) => Kind::Own { rel: *r, delta: false, cols },
                    Src::FullMulti(..) => return Err("an aggregate over the component's own relation by several indexes".into()),
                };
                stages.push(PStage { first: i, span: 1, kind, desc: d, npassed: 0 });
                i += 1;
            }
            Step::Test { op, s, matcher, want, .. } => {
                // A text test with constant patterns, of a slot an input relation's column bound: its verdicts per
                // distinct symbol of the column, made on the device by the test's automaton (`text::table`).
                let Some(m) = matcher else { return Err(format!("the text test @{op} of a pattern that is not a constant has no device form")) };
                let Key::Slot(x) = *s else { return Err(format!("the text test @{op} of a constant has no device form")) };
                let Some((rel, col)) = origin.get(x).cloned().flatten() else {
                    return Err(format!("the text test @{op} of a slot no input relation's column bound has no device form"));
                };
                let table = crate::code_rule_dsl::text::table::table(&m.plans).map_err(|e| format!("the text test @{op} has no device form: {e}"))?;
                let key = m.plans.iter().map(|p| p.key.0.as_str()).collect::<Vec<_>>().join(",");
                let mut d = mj::Stage { mode: mj::TEXT, ..mj::Stage::default() };
                d.agg = [x as u32, u32::from(*want)];
                stages.push(PStage { first: i, span: 1, kind: Kind::Text { rel, col, table, key }, desc: d, npassed: 0 });
                i += 1;
            }
            Step::Let { .. } | Step::DocOf { bind: true, .. } => {
                // Consecutive binds of slots: one map, each bind reading the slots as the ones before it left them.
                let mut d = mj::Stage::default();
                let mut n = 0;
                while let Some(s) = j.steps.get(i + n) {
                    let (out, src, doc) = match *s {
                        Step::Let { a, b } => (a, b, false),
                        Step::DocOf { out, n: src, bind: true } => (out, src, true),
                        _ => break,
                    };
                    if n == mj::MAX_BINDS {
                        break;
                    }
                    d.bind_slot[n] = out as u32;
                    d.bind_idx[n] = n as u32;
                    match src {
                        Key::Slot(x) => {
                            d.bind_kind[n] = mj::BIND_COLUMN;
                            d.col_kind[n] = mj::LEFT;
                            d.col_idx[n] = x as u32;
                            d.col_doc[n] = u32::from(doc);
                        }
                        Key::Const(c) => {
                            d.bind_kind[n] = mj::BIND_PARAM;
                            d.par_kind[n] = mj::CONST;
                            d.par_const[n] = c;
                            d.par_doc[n] = u32::from(doc);
                        }
                    }
                    bound[out] = true;
                    if out >= origin.len() {
                        origin.resize(out + 1, None);
                    }
                    origin[out] = match (doc, src) { (false, Key::Slot(x)) => origin.get(x).cloned().flatten(), _ => None };
                    n += 1;
                }
                d.nbinds = n as u32;
                stages.push(PStage { first: i, span: n, kind: Kind::Map, desc: d, npassed: 0 });
                i += n;
            }
            Step::Lt { .. } | Step::Eq { .. } | Step::Ne { .. } | Step::Doc { .. } | Step::DocOf { bind: false, .. } | Step::In { .. } => {
                // The filters over the bindings: the row program `rowcode` makes for an atom's, with every slot
                // bound so far a column of its own.
                let slots: Vec<(usize, usize)> = (0..bound.len()).filter(|&s| bound[s]).map(|s| (s, s)).collect();
                let Some(f) = rowcode::fused(&slots, &j.steps[i..]) else { return Err(format!("{} has no device form", step_name(step))) };
                let mut d = mj::Stage { mode: mj::FILTER, ..mj::Stage::default() };
                program(&mut d, &f, mj::LEFT)?;
                binds(&mut d, &f, true);
                for &(s, _) in &f.binds { bound[s] = true; }
                stages.push(PStage { first: i, span: f.steps, kind: Kind::Filter, desc: d, npassed: f.steps });
                i += f.steps;
            }
            Step::Func { name: "str_cmp", args, out, bind: true, .. } => {
                // The order of two symbols' names: compared on the device a word of eight bytes at a time, their
                // texts those of the columns that bound the slots (and the constants').
                let mut d = mj::Stage { mode: mj::STRCMP, ..mj::Stage::default() };
                let (mut cols, mut consts) = (Vec::new(), Vec::new());
                for (k, a) in args.iter().enumerate() {
                    match *a {
                        Key::Slot(x) => {
                            let Some(o) = origin.get(x).cloned().flatten() else {
                                return Err("str_cmp of a slot no input relation's column bound has no device form".into());
                            };
                            if !cols.iter().any(|(r, c): &(Arc<InRel>, usize)| Arc::ptr_eq(r, &o.0) && *c == o.1) {
                                cols.push(o);
                            }
                            d.key_kind[k] = mj::SLOT;
                            d.key_slot[k] = x as u32;
                        }
                        Key::Const(c) => {
                            consts.push(c);
                            d.key_kind[k] = mj::CONST;
                            d.key_const[k] = c;
                        }
                    }
                }
                d.bind_slot[0] = *out as u32;
                if *out >= bound.len() {
                    bound.resize(*out + 1, false);
                }
                bound[*out] = true;
                stages.push(PStage { first: i, span: 1, kind: Kind::Strcmp { cols, consts }, desc: d, npassed: 0 });
                i += 1;
            }
            Step::Func { name, kinds, args, out, bind } => {
                // Integer arithmetic: a filter of its own whose bind computes the value (`mj::BIND_ARITH`); a row
                // where it has none is dropped, as on the CPU.
                let op = arith_op(name).filter(|_| kinds.iter().all(|k| *k == crate::code_rule_dsl::logic::strings::functions::Kind::Number) && args.len() == 2);
                let Some(op) = op else { return Err(format!("the function {name} has no device form")) };
                if !*bind {
                    return Err(format!("the function {name}'s value tested (not bound) has no device form"));
                }
                let mut d = mj::Stage { mode: mj::FILTER, ..mj::Stage::default() };
                let mut ops = [0u32; 2];
                let mut sources = 0u32;
                for (k, a) in args.iter().enumerate() {
                    match *a {
                        Key::Slot(x) => {
                            if !bound.get(x).copied().unwrap_or(false) {
                                return Err(format!("the function {name} of an unbound slot"));
                            }
                            let c = d.ncols as usize;
                            d.col_kind[c] = mj::LEFT;
                            d.col_idx[c] = x as u32;
                            d.ncols += 1;
                            ops[k] = c as u32;
                        }
                        Key::Const(v) => {
                            let p = d.nparams as usize;
                            d.par_kind[p] = mj::CONST;
                            d.par_const[p] = v;
                            d.nparams += 1;
                            ops[k] = p as u32;
                            sources |= 1 << k;
                        }
                    }
                }
                d.arith[0] = [op, ops[0], ops[1], sources];
                d.bind_slot[0] = *out as u32;
                d.bind_kind[0] = mj::BIND_ARITH;
                d.bind_idx[0] = 0;
                d.nbinds = 1;
                d.npassed = 1;
                if *out >= bound.len() {
                    bound.resize(*out + 1, false);
                }
                bound[*out] = true;
                stages.push(PStage { first: i, span: 1, kind: Kind::Filter, desc: d, npassed: 1 });
                i += 1;
            }
            other => return Err(format!("{} has no device form", step_name(other))),
        }
    }
    let mut head = Vec::new();
    let mut head_slots = Vec::new();
    for t in &j.out {
        match *t {
            Term::Slot(s) => { head.push(None); head_slots.push(s); }
            Term::Const(c) => { head.push(Some(c)); head_slots.push(0); }
            Term::Cat(..) => return Err("a head that joins text".into()),
        }
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = rustc_hash::FxHasher::default();
        spec.hash(&mut h);
        h.finish()
    };
    Ok(Plan { stages, head, head_slots, nslots: j.nslots, nsteps: j.steps.len(), key })
}

/// A relation kept on the device: its rows, and its sorts by key columns.
struct Entry {
    rel: Weak<InRel>,
    dev: Rc<mj::DevRel>,
    indexes: Vec<(Vec<usize>, Rc<mj::DevIndex>)>,
    /// Per column a text test read, its distinct symbols' names; per (column, test), the test's verdicts.
    texts: Vec<(usize, Rc<mj::DevText>)>,
    verdicts: Vec<((usize, String), Rc<mj::DevRel>)>,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Resident {
    entries: Vec<Entry>,
    /// The texts of several columns and constants together (`str_cmp`'s), by the columns and the constants.
    unions: Vec<(Vec<(Weak<InRel>, usize)>, Vec<i64>, Rc<mj::DevText>)>,
    bytes: usize,
    clock: u64,
}

thread_local! {
    static RESIDENT: RefCell<Resident> = RefCell::default();
    /// Per rule (its spec's hash) and stage: the rows out per row of the input table's space, last seen.
    static GROWTH: RefCell<HashMap<(u64, usize), f64>> = RefCell::default();
    static LAST: RefCell<Option<Report>> = const { RefCell::new(None) };
}

impl Resident {
    /// Drops entries whose relation the engine dropped, then the least recently used while over the budget.
    fn trim(&mut self, room: usize) {
        self.entries.retain(|e| e.rel.strong_count() > 0);
        self.unions.retain(|(cols, _, _)| cols.iter().all(|(r, _)| r.strong_count() > 0));
        if self.bytes() + room > RESIDENT_MAX {
            self.unions.clear();
        }
        while self.bytes() + room > RESIDENT_MAX && !self.entries.is_empty() {
            let at = self.entries.iter().enumerate().min_by_key(|(_, e)| e.used).map(|(k, _)| k).expect("an entry");
            self.entries.swap_remove(at);
        }
        self.bytes = self.bytes();
    }

    fn bytes(&self) -> usize {
        self.entries.iter().map(|e| e.bytes).sum::<usize>() + self.unions.iter().map(|u| u.2.bytes()).sum::<usize>()
    }
}

/// What a stratum's device run did, for the profile.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct Report {
    pub(in crate::code_rule_dsl) rules: u32,
    pub(in crate::code_rule_dsl) stages: u32,
    pub(in crate::code_rule_dsl) submissions: u32,
    pub(in crate::code_rule_dsl) dispatches: u32,
    /// Runs again from an overflowing stage.
    pub(in crate::code_rule_dsl) reruns: u32,
    pub(in crate::code_rule_dsl) gpu_seconds: f64,
    /// This thread's CPU: encoding (uploads and sorts' encoding included), waiting, and reading the rows back into
    /// the store.
    pub(in crate::code_rule_dsl) encode_cpu: f64,
    pub(in crate::code_rule_dsl) wait_cpu: f64,
    pub(in crate::code_rule_dsl) wait_wall: f64,
    pub(in crate::code_rule_dsl) decode_cpu: f64,
    /// Head rows left once made distinct on the device (of `rows`).
    pub(in crate::code_rule_dsl) distinct: u64,
    pub(in crate::code_rule_dsl) upload_cpu: f64,
    /// Relations uploaded (and their bytes), found resident; sorts made, found resident.
    pub(in crate::code_rule_dsl) uploads: u32,
    pub(in crate::code_rule_dsl) upload_bytes: u64,
    pub(in crate::code_rule_dsl) resident_hits: u32,
    pub(in crate::code_rule_dsl) sorts: u32,
    pub(in crate::code_rule_dsl) sort_hits: u32,
    /// Rows the heads made (before the store's set), and bindings the stages made.
    pub(in crate::code_rule_dsl) rows: u64,
    pub(in crate::code_rule_dsl) bindings: u64,
    /// The rounds a recursive component ran on the device (`fixpoint`; 0 for seeds alone).
    pub(in crate::code_rule_dsl) rounds: u32,
    /// Why the device refused the stratum (at open, or at run time: a table past its budget), which then ran on the
    /// CPU; `None` when it ran here.
    pub(in crate::code_rule_dsl) refused: Option<String>,
}

/// The report of the last stratum the device ran or refused on this thread (taken).
pub(in crate::code_rule_dsl) fn take_report() -> Option<Report> {
    LAST.with_borrow_mut(Option::take)
}

/// Notes that the device refused the stratum being opened or run: it runs on the CPU, and the profile names why.
pub(super) fn refuse(why: String) {
    LAST.with_borrow_mut(|l| *l = Some(Report { refused: Some(why), ..Report::default() }));
}

/// A rule's state in a batch: its tables by totals slot, their capacities, its words.
struct Run<'p> {
    plan: &'p Plan,
    /// Each stage's input and output slots (a pass and a map keep their input's).
    io: Vec<(usize, usize)>,
    tables: Vec<Option<mj::Table>>,
    caps: Vec<usize>,
    totals: mj::Words,
    counters: mj::Words,
    /// Each stage's first counter (`None`: it counts no fused steps).
    counter: Vec<Option<usize>>,
    rels: Vec<Option<(Rc<mj::DevRel>, Option<Rc<mj::DevIndex>>)>>,
    /// The head's distinct rows (a column per slot of `uniq_cols`), made on the device after the last stage; their
    /// count is `totals`' last word.
    uniq: Option<mj::Table>,
    uniq_cols: Vec<usize>,
}

/// The device's copy of `rel`, and its sort by `cols` (none when it has no keys), uploaded and encoded into `batch`
/// when not resident.
fn resident(rel: &Arc<InRel>, cols: &[usize], batch: &mut mj::Batch, rep: &mut Report) -> Result<(Rc<mj::DevRel>, Option<Rc<mj::DevIndex>>), String> {
    RESIDENT.with_borrow_mut(|res| {
        res.clock += 1;
        let clock = res.clock;
        let weak = Arc::downgrade(rel);
        let at = res.entries.iter().position(|e| Weak::ptr_eq(&e.rel, &weak));
        let at = match at {
            Some(at) => {
                rep.resident_hits += 1;
                at
            }
            None => {
                let c0 = thread_cpu();
                let n_all = rel.len();
                let ids: Option<Vec<usize>> = (rel.ndead > 0).then(|| (0..n_all).filter(|&id| !is_dead(&rel.dead, id as u32)).collect());
                let n = ids.as_ref().map_or(n_all, Vec::len);
                let k = rel.k;
                res.trim(8 * k * n);
                // The process's device budget short (the syntax side's tapes hold most of it): every relation kept
                // and the pool's free buffers give theirs back first.
                if mj::device_bytes_left() < 8 * k * n + HEADROOM {
                    res.entries.clear();
                    res.bytes = 0;
                    mj::give_back();
                }
                let dev = mj::DevRel::upload(k, n, |v| {
                    let row_of = |r: usize| ids.as_ref().map_or(r, |ids| ids[r]);
                    if rel.view.is_none() {
                        for r in 0..n {
                            let row = rel.row(row_of(r));
                            for c in 0..k {
                                v[c * n + r] = row[c];
                            }
                        }
                    } else {
                        for r in 0..n {
                            let row = rel.at(row_of(r)).read();
                            for c in 0..k {
                                v[c * n + r] = row.at(c);
                            }
                        }
                    }
                })
                .map_err(|e| format!("{e:?}: an upload of {n} rows, {k} columns"))?;
                rep.uploads += 1;
                rep.upload_bytes += dev.bytes() as u64;
                rep.upload_cpu += thread_cpu() - c0;
                let bytes = dev.bytes();
                res.entries.push(Entry { rel: weak, dev: Rc::new(dev), indexes: Vec::new(), texts: Vec::new(), verdicts: Vec::new(), bytes, used: clock });
                res.bytes += bytes;
                res.entries.len() - 1
            }
        };
        let e = &mut res.entries[at];
        e.used = clock;
        let dev = e.dev.clone();
        if cols.is_empty() {
            return Ok((dev, None));
        }
        if let Some((_, ix)) = e.indexes.iter().find(|(c, _)| c == cols) {
            rep.sort_hits += 1;
            return Ok((dev, Some(ix.clone())));
        }
        let ix = Rc::new(batch.index(&dev, cols).map_err(|e| format!("{e:?}: a sort of {} rows", dev.rows))?);
        rep.sorts += 1;
        e.bytes += ix.bytes();
        e.indexes.push((cols.to_vec(), ix.clone()));
        Ok((dev, Some(ix)))
    })
}

/// The verdicts of text test `table` (keyed `key`) for the distinct symbols of column `col` of `rel`: a bitset by
/// symbol, 64 a row, made on the device (`Batch::verdicts`) the first time a stage asks and kept with the relation.
/// The column's symbols' names are copied to the device once (the host evaluates nothing).
fn verdicts(rel: &Arc<InRel>, col: usize, table: &Arc<crate::code_rule_dsl::text::table::Table>, key: &str, batch: &mut mj::Batch, rep: &mut Report, names: &Interner)
    -> Result<Rc<mj::DevRel>, String> {
    // The relation resident first (its entry holds the verdicts).
    resident(rel, &[], batch, rep)?;
    RESIDENT.with_borrow_mut(|res| {
        let weak = Arc::downgrade(rel);
        let e = res.entries.iter_mut().find(|e| Weak::ptr_eq(&e.rel, &weak)).expect("the relation made resident");
        if let Some((_, v)) = e.verdicts.iter().find(|((c, k), _)| *c == col && k == key) {
            rep.sort_hits += 1;
            return Ok(v.clone());
        }
        let text = match e.texts.iter().find(|(c, _)| *c == col) {
            Some((_, t)) => t.clone(),
            None => {
                let c0 = thread_cpu();
                let ids = column_symbols(rel, col);
                let t = Rc::new(mj::DevText::upload(&ids, |v, out| out.extend_from_slice(names.name(v).as_bytes())).map_err(|e| format!("{e:?}: the names of {} symbols", ids.len()))?);
                rep.uploads += 1;
                rep.upload_bytes += t.bytes() as u64;
                rep.upload_cpu += thread_cpu() - c0;
                e.bytes += t.bytes();
                e.texts.push((col, t.clone()));
                t
            }
        };
        let dt = mj::DevTable::upload(table).map_err(|e| format!("{e:?}"))?;
        let words = text.span.div_ceil(64).max(1);
        let bits = Rc::new(mj::DevRel::upload(1, words, |v| v.fill(0)).map_err(|e| format!("{e:?}: verdicts of {} symbols", text.span))?);
        batch.verdicts(&text, &dt, &bits).map_err(|e| format!("{e:?}"))?;
        batch.hold_table(dt);
        rep.sorts += 1;
        e.bytes += bits.bytes();
        e.verdicts.push(((col, key.to_owned()), bits.clone()));
        Ok(bits)
    })
}

/// Column `col`'s distinct values, of its live rows.
fn column_symbols(rel: &InRel, col: usize) -> Vec<i64> {
    let mut seen = std::collections::HashSet::new();
    let mut ids = Vec::new();
    for id in 0..rel.len() {
        if is_dead(&rel.dead, id as u32) {
            continue;
        }
        let v = if rel.view.is_none() { rel.row(id)[col] } else { rel.at(id).read().at(col) };
        if seen.insert(v) {
            ids.push(v);
        }
    }
    ids
}

/// The texts of columns `cols`' symbols and of `consts`, on the device together (`str_cmp` reads them), copied once
/// and kept while the relations are.
fn texts(cols: &[(Arc<InRel>, usize)], consts: &[i64], rep: &mut Report, names: &Interner) -> Result<Rc<mj::DevText>, String> {
    RESIDENT.with_borrow_mut(|res| {
        let found = res.unions.iter().find(|(cs, ks, _)| {
            ks == consts && cs.len() == cols.len() && cs.iter().zip(cols).all(|((w, c), (r, c2))| c == c2 && Weak::ptr_eq(w, &Arc::downgrade(r)))
        });
        if let Some((_, _, t)) = found {
            rep.resident_hits += 1;
            return Ok(t.clone());
        }
        let c0 = thread_cpu();
        let mut ids: Vec<i64> = consts.to_vec();
        for (rel, col) in cols {
            ids.extend(column_symbols(rel, *col));
        }
        let t = Rc::new(mj::DevText::upload(&ids, |v, out| out.extend_from_slice(names.name(v).as_bytes())).map_err(|e| format!("{e:?}: the names of {} symbols", ids.len()))?);
        rep.uploads += 1;
        rep.upload_bytes += t.bytes() as u64;
        rep.upload_cpu += thread_cpu() - c0;
        res.trim(t.bytes());
        res.unions.push((cols.iter().map(|(r, c)| (Arc::downgrade(r), *c)).collect(), consts.to_vec(), t.clone()));
        res.bytes = res.bytes();
        Ok(t)
    })
}

impl<'p> Run<'p> {
    fn new(plan: &'p Plan) -> Result<Run<'p>, String> {
        let mut io = Vec::new();
        let mut slot = 0;
        let mut counter = Vec::new();
        let mut ncounters = 0;
        for s in &plan.stages {
            match s.kind {
                Kind::Pass | Kind::Map => io.push((slot, slot)),
                Kind::Filter | Kind::Atom { .. } | Kind::Own { .. } | Kind::Text { .. } | Kind::Strcmp { .. } => {
                    io.push((slot, slot + 1));
                    slot += 1;
                }
            }
            if s.desc.mode == mj::AGG {
                // Its word: set when a sum passed 64 bits.
                counter.push(Some(ncounters));
                ncounters += 1;
            } else if s.npassed > 0 {
                counter.push(Some(ncounters));
                ncounters += 1 + s.npassed;
            } else {
                counter.push(None);
            }
        }
        let totals = mj::Words::zeroed(slot + 2).map_err(|e| format!("{e:?}"))?;
        totals.set(0, 1);
        let counters = mj::Words::zeroed(ncounters.max(1)).map_err(|e| format!("{e:?}"))?;
        let mut tables: Vec<Option<mj::Table>> = (0..=slot).map(|_| None).collect();
        tables[0] = Some(mj::Table::one(plan.nslots, &[]).map_err(|e| format!("{e:?}"))?);
        let mut caps = vec![0; slot + 1];
        caps[0] = 1;
        let mut uniq_cols: Vec<usize> = Vec::new();
        for (h, &s) in plan.head.iter().zip(&plan.head_slots) {
            if h.is_none() && !uniq_cols.contains(&s) {
                uniq_cols.push(s);
            }
        }
        if uniq_cols.len() > 16 {
            return Err("a head of more than 16 slots".into());
        }
        Ok(Run { plan, io, tables, caps, totals, counters, counter, rels: vec![None; plan.stages.len()], uniq: None, uniq_cols })
    }

    /// The space for stage `k`'s rows out, from its input's space.
    fn guess(&self, k: usize, cap_in: usize, nr: usize) -> usize {
        let s = &self.plan.stages[k];
        match &s.kind {
            Kind::Atom { .. } | Kind::Own { .. } if s.desc.mode == mj::JOIN => {
                let most = cap_in.saturating_mul(nr.max(1));
                if s.desc.nkeys == 0 && cap_in == 1 {
                    return nr.max(1);
                }
                let seen = GROWTH.with_borrow(|g| g.get(&(self.plan.key, k)).copied());
                let want = seen.map_or(cap_in.saturating_mul(2), |r| ((cap_in as f64 * r * 1.25).ceil() as usize).max(16));
                want.min(most).clamp(1, GUESS_MAX.max(cap_in))
            }
            _ => cap_in,
        }
    }

    /// Encodes stages `from..` into `batch`; the component's own relations are `own`'s.
    fn encode(&mut self, from: usize, batch: &mut mj::Batch, rep: &mut Report, own: &mut Own, names: &Interner) -> Result<(), String> {
        for k in from..self.plan.stages.len() {
            let s = &self.plan.stages[k];
            let (i, o) = self.io[k];
            match &s.kind {
                Kind::Pass => {}
                Kind::Map => {
                    let t = self.tables[i].as_ref().expect("the map's table");
                    batch.map(s.desc, t, i, &self.totals).map_err(|e| format!("{e:?}"))?;
                }
                Kind::Filter | Kind::Atom { .. } | Kind::Own { .. } | Kind::Text { .. } | Kind::Strcmp { .. } => {
                    match (&s.kind, &self.rels[k]) {
                        (Kind::Strcmp { cols, consts }, None) => self.rels[k] = Some((texts(cols, consts, rep, names)?.rel(), None)),
                        (Kind::Atom { rel, cols }, None) => self.rels[k] = Some(resident(rel, cols, batch, rep)?),
                        (Kind::Text { rel, col, table, key }, None) => self.rels[k] = Some((verdicts(rel, *col, table, key, batch, rep, names)?, None)),
                        (Kind::Own { rel, delta, cols }, None) => {
                            self.rels[k] = Some(own.get(*rel, *delta, cols, batch, rep)?.ok_or("an own relation with no rows")?);
                        }
                        _ => {}
                    }
                    let right = self.rels[k].as_ref().map(|(r, ix)| (&**r, ix.as_deref()));
                    let cap_in = self.caps[i];
                    if self.caps[o] == 0 {
                        self.caps[o] = self.guess(k, cap_in, right.map_or(0, |r| r.0.rows));
                    }
                    let cap = self.caps[o];
                    if 8 * self.plan.nslots.max(1) * cap > TABLE_MAX {
                        return Err(format!("a binding table of {cap} rows past the device's budget"));
                    }
                    let out = mj::Table::new(self.plan.nslots, cap).map_err(|e| format!("{e:?}: a binding table of {cap} rows, {} slots", self.plan.nslots))?;
                    if let Some(c) = self.counter[k] {
                        for w in c..c + 1 + if s.desc.mode == mj::AGG { 0 } else { s.npassed } {
                            self.counters.set(w, 0);
                        }
                    }
                    let left = self.tables[i].as_ref().expect("the stage's input");
                    batch.stage(s.desc, left, right, i, o, &self.totals, &self.counters, self.counter[k], &out).map_err(|e| format!("{e:?}: a stage's space for {cap_in} rows"))?;
                    self.tables[o] = Some(out);
                }
            }
        }
        // The head rows made distinct, as the store keeps them (fewer rows back to the host).
        let last = self.io.last().map_or(0, |io| io.1);
        let table = self.tables[last].as_ref().expect("the last table");
        let uniq = mj::Table::new(self.uniq_cols.len(), table.cap).map_err(|e| format!("{e:?}: the head's table of {} rows", table.cap))?;
        batch.dedup(table, &self.uniq_cols, last, self.totals.len - 1, &self.totals, &uniq).map_err(|e| format!("{e:?}"))?;
        self.uniq = Some(uniq);
        Ok(())
    }

    /// Why its rows are not the CPU's, once it ran: an aggregate's sum past 64 bits (the CPU's fold holds 128).
    fn failed(&self) -> Option<String> {
        self.plan.stages.iter().zip(&self.counter).any(|(s, c)| s.desc.mode == mj::AGG && c.is_some_and(|c| self.counters.get(c) != 0))
            .then(|| "an aggregate's sum past 64 bits".to_owned())
    }

    /// The device bytes of its tables.
    fn bytes(&self) -> usize {
        self.tables.iter().chain(std::iter::once(&self.uniq)).flatten().map(mj::Table::bytes).sum()
    }

    /// The first stage whose rows overflowed its table, with its true count; the counts of every stage before are
    /// final.
    fn overflow(&self) -> Option<(usize, usize)> {
        self.plan.stages.iter().enumerate().find_map(|(k, _)| {
            let (i, o) = self.io[k];
            if i == o {
                return None;
            }
            let n = self.totals.get(o) as usize;
            (n > self.caps[o]).then_some((k, n))
        })
    }
}

/// Runs the seed joins `rules` (each: its plan, its rule, its head relation) on the device, their head rows into
/// `fresh` as the CPU's run would add them. Returns each rule's head rows (before the store's set). The rules go in
/// as few submissions as their tables' space allows (`BATCH_MAX`): one, for most strata.
pub(super) fn run(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    on_lane("stratum joins", rules.len(), || run_here(rules, store, names, fresh))
}

/// `f`, a stratum's work on the device, as one job of the Metal binding's GPU lane (`Executor::inline`): its
/// submissions on the lane's scoreboard with the device's seconds as its `gpu run` phase.
fn on_lane<R>(form: &'static str, rules: usize, f: impl FnOnce() -> Result<R, String>) -> Result<R, String> {
    let ex = crate::code_rule_dsl::logic::lanes::executor_for(true);
    ex.inline(rerail_accel::Lane::Gpu, form, rules as f64, || {
        let r = f();
        if let Some(gpu) = LAST.with_borrow(|l| l.as_ref().map(|r| r.gpu_seconds)) {
            rerail_accel::score::add("gpu run", 0.0, gpu);
        }
        r
    })
}

fn run_here(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "device joins".into());
    let c0 = thread_cpu();
    let mut rep = Report { rules: rules.len() as u32, stages: rules.iter().map(|r| r.0.stages.len() as u32).sum(), ..Report::default() };
    let mut out = Vec::with_capacity(rules.len());
    let mut from = 0;
    let mut cleared = false;
    while from < rules.len() {
        match group(&rules[from..], store, names, fresh, &mut rep) {
            Ok(rows) => {
                from += rows.len();
                out.extend(rows);
            }
            // Out of device buffers: the relations kept resident and the pool's free buffers give theirs back, once,
            // and the group runs again.
            Err(e) if !cleared && e.contains("BackendCapacity") => {
                RESIDENT.with_borrow_mut(|r| {
                    r.entries.clear();
                    r.bytes = 0;
                });
                mj::give_back();
                cleared = true;
            }
            Err(e) => return Err(e),
        }
    }
    rep.encode_cpu = (thread_cpu() - c0 - rep.wait_cpu - rep.decode_cpu - rep.upload_cpu).max(0.0);
    let rows: u64 = rep.rows;
    crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "head rows read back", rows);
    LAST.with_borrow_mut(|l| *l = Some(rep));
    Ok(out)
}

/// The first rules of `rules` that fit one submission's space, run: their head rows as `run`'s.
fn group(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh], rep: &mut Report) -> Result<Vec<u64>, String> {
    let mut batch = mj::Batch::new().map_err(|e| format!("{e:?}"))?;
    let mut runs: Vec<Run<'_>> = Vec::with_capacity(rules.len());
    {
        let _h = crate::code_rule_dsl::logic::lanes::prof::host("device join (decision 0095)", "encode: uploads, sorts, stages");
        let mut bytes = 0;
        for (plan, _, _) in rules {
            if !runs.is_empty() && (bytes > BATCH_MAX || mj::device_bytes_left() < HEADROOM) {
                break;
            }
            let mut r = Run::new(plan)?;
            r.encode(0, &mut batch, rep, &mut Own::default(), names)?;
            bytes += r.bytes();
            runs.push(r);
        }
    }
    let mut pending: Vec<usize> = (0..runs.len()).collect();
    loop {
        let (t0, w0) = (Instant::now(), thread_cpu());
        let done = batch.finish().map_err(|e| format!("{e:?}"))?;
        rep.wait_wall += t0.elapsed().as_secs_f64();
        rep.wait_cpu += thread_cpu() - w0;
        rep.submissions += done.submissions;
        rep.dispatches += done.dispatches;
        rep.gpu_seconds += done.gpu_seconds;
        let mut again = Vec::new();
        for &k in &pending {
            if let Some((s, n)) = runs[k].overflow() {
                let o = runs[k].io[s].1;
                runs[k].caps[o] = n;
                // The stages after it are sized again from the exact count.
                for later in s + 1..runs[k].plan.stages.len() {
                    let (i, o2) = runs[k].io[later];
                    if i != o2 {
                        runs[k].caps[o2] = 0;
                    }
                }
                again.push((k, s));
            }
        }
        if again.is_empty() {
            if let Some(why) = runs.iter().find_map(Run::failed) {
                return Err(why);
            }
            break;
        }
        rep.reruns += again.len() as u32;
        pending = again.iter().map(|&(k, _)| k).collect();
        for (k, s) in again {
            runs[k].encode(s, &mut batch, rep, &mut Own::default(), names)?;
        }
    }
    // The rows into the store, and each step's counts.
    let d0 = thread_cpu();
    let _h = crate::code_rule_dsl::logic::lanes::prof::host("device join (decision 0095)", "decode: rows into the store");
    let mut out = Vec::with_capacity(runs.len());
    for (r, &(plan, j, head)) in runs.iter().zip(rules) {
        let last = r.io.last().map_or(0, |io| io.1);
        let n = r.totals.get(last) as usize;
        let distinct = r.totals.get(r.totals.len - 1) as usize;
        let uniq = r.uniq.as_ref().expect("the head's distinct rows");
        let cols: Vec<&[i64]> = plan
            .head_slots
            .iter()
            .map(|s| uniq.column(r.uniq_cols.iter().position(|u| u == s).unwrap_or(0), distinct))
            .collect();
        let mut row = vec![0i64; plan.head.len()];
        let rel = &store.rels[head];
        rep.distinct += distinct as u64;
        for x in 0..distinct {
            for (c, h) in plan.head.iter().enumerate() {
                row[c] = h.unwrap_or_else(|| cols[c][x]);
            }
            if !rel.contains(&row) {
                fresh[head].add(&row);
            }
        }
        out.push(n as u64);
        rep.rows += n as u64;
        counted(r, plan, j, rep);
    }
    drop(_h);
    rep.decode_cpu += thread_cpu() - d0;
    Ok(out)
}


/// The most rounds a recursive component runs on the device before it is refused (and runs on the CPU).
const ROUNDS_MAX: usize = 4096;

/// A recursive component's relations on the device, between its rounds (`fixpoint`): each one's rows so far and its
/// last round's new rows, and their sorts by the key columns the probes read them by.
#[derive(Default)]
struct Own {
    total: HashMap<usize, Rc<mj::DevRel>>,
    delta: HashMap<usize, Rc<mj::DevRel>>,
    sorts: HashMap<(usize, bool, Vec<usize>), Rc<mj::DevIndex>>,
}

impl Own {
    /// Relation `rel`'s rows (`delta`: its last round's) and their sort by `cols`, encoded into `batch` the first
    /// time a round asks; `None` when it has no rows.
    fn get(&mut self, rel: usize, delta: bool, cols: &[usize], batch: &mut mj::Batch, rep: &mut Report) -> Result<Option<(Rc<mj::DevRel>, Option<Rc<mj::DevIndex>>)>, String> {
        let Some(dev) = (if delta { &self.delta } else { &self.total }).get(&rel).cloned() else { return Ok(None) };
        if dev.rows == 0 {
            return Ok(None);
        }
        if cols.is_empty() {
            return Ok(Some((dev, None)));
        }
        let key = (rel, delta, cols.to_vec());
        if let Some(ix) = self.sorts.get(&key) {
            rep.sort_hits += 1;
            return Ok(Some((dev, Some(ix.clone()))));
        }
        let ix = Rc::new(batch.index(&dev, cols).map_err(|e| format!("{e:?}: a sort of {} rows", dev.rows))?);
        rep.sorts += 1;
        self.sorts.insert(key, ix.clone());
        Ok(Some((dev, Some(ix))))
    }

    /// Whether every own relation `plan` reads has rows: a probe over one without reads nothing, and is not run.
    fn ready(&self, plan: &Plan) -> bool {
        plan.stages.iter().all(|s| match &s.kind {
            Kind::Own { rel, delta, .. } => (if *delta { &self.delta } else { &self.total }).get(rel).is_some_and(|d| d.rows > 0),
            _ => true,
        })
    }
}

/// Whether `plan` reads the component's own relations (a delta probe's plan).
pub(super) fn reads_own(plan: &Plan) -> bool {
    plan.stages.iter().any(|s| matches!(s.kind, Kind::Own { .. }))
}

/// One round's rules run in one batch: each one's head rows made distinct on the device, with their count. A rule
/// over an own relation with no rows is not run (`None`).
fn round<'p>(rules: &[(&'p Plan, &JoinRule, usize)], own: &mut Own, names: &Interner, rep: &mut Report) -> Result<Vec<Option<(Run<'p>, usize)>>, String> {
    let mut batch = mj::Batch::new().map_err(|e| format!("{e:?}"))?;
    let mut runs: Vec<Option<Run<'p>>> = Vec::with_capacity(rules.len());
    for (plan, _, _) in rules {
        if !own.ready(plan) {
            runs.push(None);
            continue;
        }
        let mut r = Run::new(plan)?;
        r.encode(0, &mut batch, rep, own, names)?;
        runs.push(Some(r));
    }
    let mut pending: Vec<usize> = (0..runs.len()).filter(|&k| runs[k].is_some()).collect();
    loop {
        let (t0, w0) = (Instant::now(), thread_cpu());
        let done = batch.finish().map_err(|e| format!("{e:?}"))?;
        rep.wait_wall += t0.elapsed().as_secs_f64();
        rep.wait_cpu += thread_cpu() - w0;
        rep.submissions += done.submissions;
        rep.dispatches += done.dispatches;
        rep.gpu_seconds += done.gpu_seconds;
        let mut again = Vec::new();
        for &k in &pending {
            let r = runs[k].as_mut().expect("a run");
            if let Some((s, n)) = r.overflow() {
                let o = r.io[s].1;
                r.caps[o] = n;
                for later in s + 1..r.plan.stages.len() {
                    let (i, o2) = r.io[later];
                    if i != o2 {
                        r.caps[o2] = 0;
                    }
                }
                again.push((k, s));
            }
        }
        if again.is_empty() {
            if let Some(why) = runs.iter().flatten().find_map(Run::failed) {
                return Err(why);
            }
            break;
        }
        rep.reruns += again.len() as u32;
        pending = again.iter().map(|&(k, _)| k).collect();
        for (k, s) in again {
            runs[k].as_mut().expect("a run").encode(s, &mut batch, rep, own, names)?;
        }
    }
    let mut out = Vec::with_capacity(runs.len());
    for (r, &(plan, j, _)) in runs.into_iter().zip(rules) {
        out.push(r.map(|r| {
            let last = r.io.last().map_or(0, |io| io.1);
            rep.rows += u64::from(r.totals.get(last));
            counted(&r, plan, j, rep);
            let distinct = r.totals.get(r.totals.len - 1) as usize;
            (r, distinct)
        }));
    }
    Ok(out)
}

/// A recursive component's fixpoint on the device: its seed joins `seeds` once, then its delta joins `deltas` in
/// rounds, each round's new rows (made distinct, less the rows the relation has) its next delta, until a round makes
/// none. Each relation's rows stay on the device between rounds; the host reads one count per relation a round, and
/// the new rows once, at the end, into `fresh` as the CPU's waves would add them. `heads` are the component's
/// relations, which must be empty when it starts. Returns each rule's rows made, seeds then deltas.
pub(super) fn fixpoint(seeds: &[(&Plan, &JoinRule, usize)], deltas: &[(&Plan, &JoinRule, usize)], heads: &[usize], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    on_lane("stratum rounds", seeds.len() + deltas.len(), || fixpoint_here(seeds, deltas, heads, store, names, fresh))
}

fn fixpoint_here(seeds: &[(&Plan, &JoinRule, usize)], deltas: &[(&Plan, &JoinRule, usize)], heads: &[usize], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "device fixpoint".into());
    let c0 = thread_cpu();
    let mut rep = Report { rules: (seeds.len() + deltas.len()) as u32, stages: seeds.iter().chain(deltas).map(|r| r.0.stages.len() as u32).sum(), ..Report::default() };
    for &h in heads {
        let rel = &store.rels[h];
        if rel.n > 0 {
            return Err("a recursive component whose relations have rows before its run".into());
        }
        if rel.k > mj::MAX_KEYS {
            return Err(format!("a recursive relation of {} columns (the device's keys hold {})", rel.k, mj::MAX_KEYS));
        }
    }
    let mut own = Own::default();
    let mut made = vec![0u64; seeds.len() + deltas.len()];
    let mut rounds = 0;
    let mut first = true;
    loop {
        rounds += 1;
        if rounds > ROUNDS_MAX {
            return Err(format!("a recursive component past {ROUNDS_MAX} rounds"));
        }
        let (rules, base) = if first { (seeds, 0) } else { (deltas, seeds.len()) };
        let ran = round(rules, &mut own, names, &mut rep)?;
        first = false;
        // Each head's rows this round, from every rule that made some, into one union; made distinct; less the
        // relation's rows: its new rows.
        let mut batch = mj::Batch::new().map_err(|e| format!("{e:?}"))?;
        let mut news: Vec<(usize, mj::Table, mj::Words, bool)> = Vec::new();
        // What the encoded work reads, held until it completes (a pooled buffer dropped early is written again).
        let mut hold: Vec<mj::Table> = Vec::new();
        for &h in heads {
            let k = store.rels[h].k;
            let from: Vec<(usize, &Run<'_>, usize)> = ran.iter().enumerate().filter_map(|(x, r)| {
                let (r, n) = r.as_ref()?;
                (rules[x].2 == h && *n > 0).then_some((x, r, *n))
            }).collect();
            let cap: usize = from.iter().map(|f| f.2).sum();
            for (x, _, n) in &from {
                made[base + x] += *n as u64;
            }
            if cap == 0 {
                continue;
            }
            let union = mj::Table::new(k, cap).map_err(|e| format!("{e:?}: a union of {cap} rows"))?;
            let mut at = 0;
            for (x, r, n) in &from {
                let plan = rules[*x].0;
                let cols: Vec<mj::Col> = plan.head.iter().zip(&plan.head_slots).map(|(c, s)| match c {
                    Some(v) => mj::Col::Const(*v),
                    None => mj::Col::From(r.uniq_cols.iter().position(|u| u == s).unwrap_or(0)),
                }).collect();
                batch.copy_rows(mj::Rows::Table(r.uniq.as_ref().expect("the head's rows")), *n, &cols, mj::Rows::Table(&union), at).map_err(|e| format!("{e:?}"))?;
                at += n;
            }
            let words = mj::Words::zeroed(3).map_err(|e| format!("{e:?}"))?;
            words.set(0, cap as u32);
            let distinct = mj::Table::new(k, cap).map_err(|e| format!("{e:?}"))?;
            let all: Vec<usize> = (0..k).collect();
            batch.dedup(&union, &all, 0, 1, &words, &distinct).map_err(|e| format!("{e:?}"))?;
            match own.get(h, false, &all, &mut batch, &mut rep)? {
                Some((total, ix)) => {
                    let mut d = mj::Stage { mode: mj::ANTI, ..mj::Stage::default() };
                    for c in 0..k {
                        d.key_rcol[c] = c as u32;
                        d.key_kind[c] = mj::SLOT;
                        d.key_slot[c] = c as u32;
                    }
                    d.nkeys = k as u32;
                    let fresh_rows = mj::Table::new(k, cap).map_err(|e| format!("{e:?}"))?;
                    let counters = mj::Words::zeroed(1).map_err(|e| format!("{e:?}"))?;
                    batch.stage(d, &distinct, Some((&total, ix.as_deref())), 1, 2, &words, &counters, None, &fresh_rows).map_err(|e| format!("{e:?}"))?;
                    news.push((h, fresh_rows, words, true));
                    hold.push(distinct);
                }
                None => news.push((h, distinct, words, false)),
            }
            hold.push(union);
        }
        let (t0, w0) = (Instant::now(), thread_cpu());
        let done = batch.finish().map_err(|e| format!("{e:?}"))?;
        rep.wait_wall += t0.elapsed().as_secs_f64();
        rep.wait_cpu += thread_cpu() - w0;
        rep.submissions += done.submissions;
        rep.dispatches += done.dispatches;
        rep.gpu_seconds += done.gpu_seconds;
        drop(ran);
        drop(hold);
        // The new rows onto each relation, and as its next delta; a relation with none has no delta.
        let mut batch = mj::Batch::new().map_err(|e| format!("{e:?}"))?;
        let mut any = false;
        let mut old: Vec<Rc<mj::DevRel>> = std::mem::take(&mut own.delta).into_values().collect();
        let mut rows_held: Vec<mj::Table> = Vec::new();
        for (h, rows, words, anti) in news {
            let n = words.get(if anti { 2 } else { 1 }) as usize;
            if n == 0 {
                continue;
            }
            any = true;
            let k = store.rels[h].k;
            let ident: Vec<mj::Col> = (0..k).map(mj::Col::From).collect();
            let had = own.total.get(&h).map_or(0, |t| t.rows);
            let total = mj::DevRel::empty(k, had + n).map_err(|e| format!("{e:?}: a relation of {} rows", had + n))?;
            if let Some(old) = own.total.get(&h) {
                batch.copy_rows(mj::Rows::Rel(old), had, &ident, mj::Rows::Rel(&total), 0).map_err(|e| format!("{e:?}"))?;
            }
            batch.copy_rows(mj::Rows::Table(&rows), n, &ident, mj::Rows::Rel(&total), had).map_err(|e| format!("{e:?}"))?;
            let delta = mj::DevRel::empty(k, n).map_err(|e| format!("{e:?}"))?;
            batch.copy_rows(mj::Rows::Table(&rows), n, &ident, mj::Rows::Rel(&delta), 0).map_err(|e| format!("{e:?}"))?;
            old.extend(own.total.insert(h, Rc::new(total)));
            own.delta.insert(h, Rc::new(delta));
            own.sorts.retain(|(r, _, _), _| *r != h);
            rows_held.push(rows);
        }
        let done = batch.finish().map_err(|e| format!("{e:?}"))?;
        drop((old, rows_held));
        rep.submissions += done.submissions;
        rep.dispatches += done.dispatches;
        rep.gpu_seconds += done.gpu_seconds;
        if !any {
            break;
        }
    }
    // The rows into the store, once.
    let d0 = thread_cpu();
    for &h in heads {
        let Some(total) = own.total.get(&h) else { continue };
        let k = store.rels[h].k;
        let cols: Vec<&[i64]> = (0..k).map(|c| total.column(c)).collect();
        let mut row = vec![0i64; k];
        for x in 0..total.rows {
            for c in 0..k {
                row[c] = cols[c][x];
            }
            fresh[h].add(&row);
        }
        rep.distinct += total.rows as u64;
    }
    rep.decode_cpu += thread_cpu() - d0;
    rep.rounds = rounds as u32;
    rep.encode_cpu = (thread_cpu() - c0 - rep.wait_cpu - rep.decode_cpu - rep.upload_cpu).max(0.0);
    LAST.with_borrow_mut(|l| *l = Some(rep));
    Ok(made)
}

/// Each step's observed counts after `r` ran, as the CPU's walk makes them, into `j`'s (the replanner's fanouts),
/// and the growth seen, for the next chunk's guesses.
fn counted(r: &Run<'_>, plan: &Plan, j: &JoinRule, rep: &mut Report) {
    // Counts as the CPU's walk makes them: each stage's entries are the rows out of the stage before.
    let get = |slot: usize| if slot == 0 { 1u64 } else { u64::from(r.totals.get(slot)) };
    if plan.nsteps > 0 {
        let c = &j.counts[0][0];
        c.set(c.get() + 1);
    }
    for (k, s) in plan.stages.iter().enumerate() {
        let (i, o) = r.io[k];
        let n_in = get(i);
        let n_out = get(o);
        rep.bindings += if i != o { n_out } else { 0 };
        let vals: Vec<u64> = match (&s.kind, r.counter[k]) {
            (Kind::Pass | Kind::Map, _) => vec![n_in; s.span],
            _ if s.desc.mode == mj::AGG => vec![n_out],
            (Kind::Atom { .. } | Kind::Own { .. }, Some(c)) => (0..=s.npassed).map(|m| u64::from(r.counters.get(c + m))).collect(),
            (Kind::Filter, Some(c)) => (1..=s.npassed).map(|m| u64::from(r.counters.get(c + m))).collect(),
            _ => vec![n_out],
        };
        for (m, v) in vals.into_iter().enumerate() {
            let at = s.first + m;
            let c = &j.counts[at][1];
            c.set(c.get() + v);
            if let Some(next) = j.counts.get(at + 1) {
                next[0].set(next[0].get() + v);
            }
        }
        // The growth seen, for the next chunk's guess.
        if i != o && r.caps[i] > 0 {
            GROWTH.with_borrow_mut(|g| g.insert((plan.key, k), n_out as f64 / r.caps[i] as f64));
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    fn hex(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }

    /// One stratum's seed joins run on the CPU and on the device: the same rows, and the same observed counts per
    /// step. The joins: a scan, a keyed join with a fused comparison and `docof`, a negation, a semi-join, a filter
    /// after a step that binds nothing, a `let`, constants in keys and heads; one rule whose second join's rows
    /// overflow the first guess of its table.
    #[test]
    fn a_strata_seeds_on_the_device_are_the_cpus_rows_and_counts() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::prefilter::tests::temp();
        drop(crate::code_rule_dsl::prefilter::tests::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 7u64;
        let mut next = |m: i64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) as i64).rem_euclid(m)
        };
        db.execute_batch("CREATE TEMP TABLE a (c0, c1); CREATE TEMP TABLE b (c0, c1); CREATE TEMP TABLE c (c0);").unwrap();
        for _ in 0..3000 {
            db.execute("INSERT INTO a VALUES (?1, ?2)", [((next(4) << 32) | next(500)), next(200)]).unwrap();
        }
        for _ in 0..4000 {
            db.execute("INSERT INTO b VALUES (?1, ?2)", [next(200), ((next(4) << 32) | next(500))]).unwrap();
        }
        for _ in 0..300 {
            db.execute("INSERT INTO c VALUES (?1)", [((next(4) << 32) | next(500))]).unwrap();
        }
        let src = |t: &str, cols: &str, types: &str| format!("t:x:{}:{}:{types}", hex(&format!("temp.{t}")), hex(cols));
        let (a, b, c) = (src("a", "c0, c1", "ii"), src("b", "c0, c1", "ii"), src("c", "c0", "i"));
        let rules = [
            // h0(x, z) :- a(x, y), b(y, z), z < x, @doc(z) = @doc(x), !c(z).
            format!("pos {a} v0 v1\npos {b} v1 v2\nlt v2 v0\ndeq v2 v0\nneg {c} v2\nhead v0 v2"),
            // h0(x, 7) :- a(x, 3), c(x).
            format!("pos {a} v0 i:3\npos {c} v0\nhead v0 i:7"),
            // h0(y, w) :- a(x, y), c(x), w = y, y != 5, d = @doc(x).
            format!("pos {a} v0 v1\npos {c} v0\nne v1 i:5\neq v2 v1\ndocof v3 v0\nhead v1 v2"),
            // h0(x, z) :- a(x, y), b(y, z): many rows (past the first guess of its table).
            format!("pos {a} v0 v1\npos {b} v1 v2\nhead v0 v2"),
            // h0(x, w) :- a(x, y), s = x + y, w = s / (y - 3), w < x: integer arithmetic, a zero divisor dropped.
            format!("pos {a} v0 v1\nfunc rr_add v2 v0 v1\nfunc rr_sub v3 v1 i:3\nfunc rr_div v4 v2 v3\nlt v4 v0\nhead v0 v4"),
            // h0(x, w) :- a(x, y), w = x * x % 7, max and min: the product overflows past the first document.
            format!("pos {a} v0 v1\nfunc rr_mul v2 v0 v0\nfunc mod v3 v2 i:7\nfunc rr_max v4 v3 v1\nfunc rr_min v5 v4 i:100\nhead v0 v5"),
            // Aggregates. h0(x, n) :- c(x), n = count : { a(x, _) }.
            format!("pos {c} v0\ncount {a} v0 _ v1\nhead v0 v1"),
            // h0(x, s) :- c(x), s = sum y : { a(x, y) }.
            format!("pos {c} v0\nsum {a} v0 _ 1 v1\nhead v0 v1"),
            // h0(z, m) :- b(y, z), m = min x : { a(x, y) }: by another key; a key with no rows has no minimum.
            format!("pos {b} v0 v1\nmin {a} _ v0 0 v2\nhead v1 v2"),
            // h0(x, y) :- a(x, y), y = max y2 : { a(x, y2) }: the value tested, not bound.
            format!("pos {a} v0 v1\nmax {a} v0 _ 1 v1\nhead v0 v1"),
            // h0(x, n) :- c(x), n = count : { b(_, _) }: no keys, the whole relation.
            format!("pos {c} v0\ncount {b} _ _ v1\nhead v0 v1"),
        ];
        let mut got = Vec::new();
        for device in [false, true] {
            db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
            let mut spec = vec!["name dev_test".to_owned()];
            if device {
                spec.push("device".into());
            }
            spec.push("rel h0 ii empty".into());
            spec.extend(rules.iter().map(|r| format!("seed 0 {}", hex(r))));
            super::super::open(&db, &spec.join("\n")).unwrap();
            super::super::run(&db, "dev_test", 0).unwrap();
            let report = super::take_report();
            assert_eq!(report.as_ref().is_some_and(|r| r.refused.is_none()), device, "{report:?}");
            if let Some(r) = &report {
                assert!(r.reruns >= 1 && r.submissions >= 2, "the large join overflows its first table: {r:?}");
            }
            let counts = super::super::step_counts(&db, "dev_test").unwrap();
            super::super::close(&db, "dev_test").unwrap();
            let mut rows: Vec<(i64, i64)> = db.prepare("SELECT c0, c1 FROM temp.h0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            rows.sort_unstable();
            got.push((rows, counts));
        }
        assert!(got[0].0.len() > 1000, "{} rows", got[0].0.len());
        assert_eq!(got[0].0, got[1].0, "the device's rows are the CPU's");
        assert_eq!(got[0].1, got[1].1, "the device's counts per step are the CPU's");
    }

    /// A recursive stratum on the device: its rounds to the fixpoint there, the same rows as the CPU's waves.
    #[test]
    fn a_recursive_stratum_on_the_device_is_the_cpus_fixpoint() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::prefilter::tests::temp();
        drop(crate::code_rule_dsl::prefilter::tests::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 11u64;
        let mut next = |m: i64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) as i64).rem_euclid(m)
        };
        db.execute_batch("CREATE TEMP TABLE e (c0, c1);").unwrap();
        // A sparse graph of chains: its closure takes many rounds.
        for v in 0..100 {
            db.execute("INSERT INTO e VALUES (?1, ?2)", [v, v + 1]).unwrap();
            if next(5) == 0 {
                db.execute("INSERT INTO e VALUES (?1, ?2)", [v, next(100)]).unwrap();
            }
        }
        let e = format!("t:x:{}:{}:ii", hex("temp.e"), hex("c0, c1"));
        // h0(x, y) :- e(x, y).   h0(x, z) :- h0(x, y), h0(y, z), z != x (both atoms recursive: not a closure's walk).
        let seed = format!("pos {e} v0 v1\nhead v0 v1");
        let left = "pos d0 v0 v1\npos c0 v1 v2\nne v2 v0\nhead v0 v2".to_owned();
        let right = "pos c0 v0 v1\npos d0 v1 v2\nne v2 v0\nhead v0 v2".to_owned();
        let mut got = Vec::new();
        let gpu_jobs = || crate::code_rule_dsl::logic::lanes::executor_for(true).score().units[rerail_accel::Lane::Gpu.index()].jobs;
        let jobs0 = gpu_jobs();
        for device in [false, true] {
            db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
            let mut spec = vec!["name dev_rec".to_owned()];
            if device {
                spec.push("device".into());
            }
            spec.push("rel h0 ii empty".into());
            spec.push(format!("seed 0 {}", hex(&seed)));
            spec.push(format!("join 0 0 {}", hex(&left)));
            spec.push(format!("join 0 0 {}", hex(&right)));
            super::super::open(&db, &spec.join("\n")).unwrap();
            super::super::run(&db, "dev_rec", 0).unwrap();
            let report = super::take_report();
            if device {
                let r = report.as_ref().expect("a device report");
                assert!(r.refused.is_none(), "{r:?}");
                assert!(r.rounds > 5, "the closure takes rounds: {r:?}");
            }
            super::super::close(&db, "dev_rec").unwrap();
            let mut rows: Vec<(i64, i64)> = db.prepare("SELECT c0, c1 FROM temp.h0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            rows.sort_unstable();
            got.push(rows);
        }
        assert!(got[0].len() > 1000, "{} rows", got[0].len());
        assert_eq!(got[0], got[1], "the device's fixpoint is the CPU's");
        assert!(gpu_jobs() > jobs0, "the device's rounds are a job on the GPU lane's scoreboard");
    }

    /// Text tests on the device: each test's verdicts per distinct symbol made by its automaton, the same rows as
    /// the CPU's matcher gives (names past one chunk of 32 bytes among them).
    #[test]
    fn text_tests_on_the_device_are_the_cpus() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::prefilter::tests::temp();
        drop(crate::code_rule_dsl::prefilter::tests::open(&store));
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 5u64;
        let mut next = |m: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) % m
        };
        let parts = ["foo", "bar", "Src", "src/lib", "test", "_", ".ts", "αβ", "x9", "TODO", "a_b", "/"];
        db.execute_batch("CREATE TEMP TABLE n (c0, c1);").unwrap();
        for k in 0..2000 {
            let len = 1 + next(if k % 50 == 0 { 30 } else { 5 });
            let name: String = (0..len).map(|_| parts[next(parts.len() as u64) as usize]).collect();
            db.execute("INSERT INTO n VALUES (?1, ?2)", rusqlite::params![name, k]).unwrap();
        }
        db.execute_batch("CREATE TEMP TABLE m (c0);").unwrap();
        for k in 0..60 {
            let len = 1 + next(4);
            let name: String = (0..len).map(|_| parts[next(parts.len() as u64) as usize]).collect();
            db.execute("INSERT INTO m VALUES (?1)", rusqlite::params![if k == 0 { String::new() } else { name }]).unwrap();
        }
        let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
        let m = format!("t:x:{}:{}:s", hex("temp.m"), hex("c0"));
        let sym = |t: &str| format!("x:{}", hex(t));
        let rules = [
            format!("pos {n} v0 v1\ntest prefix v0 {} 1\nhead v0 v1", sym("foo")),
            format!("pos {n} v0 v1\ntest contains v0 {} 0\nhead v0 v1", sym("_b")),
            format!("pos {n} v0 v1\ntest rx v0 {} 1\nhead v0 v1", sym("^[a-z]+_[a-z]+[0-9]*$")),
            format!("pos {n} v0 v1\ntestset suffix v0 1 {} {}\nhead v0 v1", sym(".ts"), sym("bar")),
            format!("pos {n} v0 v1\neq v2 v0\ntest iprefix v2 {} 1\nhead v0 v1", sym("SRC")),
            format!("pos {n} v0 v1\ntest glob v0 {} 1\nhead v0 v1", sym("src/lib*.ts")),
            format!("pos {n} v0 v1\ntest contains v0 {} 1\ntest suffix v0 {} 0\nhead v0 v1", sym("αβ"), sym("x9")),
            // Ordering by names: against a constant, and between two relations' symbols (an empty name among them).
            format!("pos {n} v0 v1\nfunc str_cmp v2 v0 {}\nlt v2 i:0\nhead v0 v1", sym("foo_bar")),
            format!("pos {m} v0\npos {n} v1 v2\nfunc str_cmp v3 v1 v0\neq v3 i:0\nhead v1 v2"),
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:40\nfunc str_cmp v3 v0 v1\nlt i:0 v3\nhead v0 v2"),
        ];
        let mut got = Vec::new();
        for device in [false, true] {
            db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
            let mut spec = vec!["name dev_text".to_owned()];
            if device {
                spec.push("device".into());
            }
            spec.push("rel h0 si empty".into());
            spec.extend(rules.iter().map(|r| format!("seed 0 {}", hex(r))));
            super::super::open(&db, &spec.join("\n")).unwrap();
            super::super::run(&db, "dev_text", 0).unwrap();
            let report = super::take_report();
            assert_eq!(report.as_ref().is_some_and(|r| r.refused.is_none()), device, "{report:?}");
            let counts = super::super::step_counts(&db, "dev_text").unwrap();
            super::super::close(&db, "dev_text").unwrap();
            let mut rows: Vec<(i64, i64)> = db.prepare("SELECT c0, c1 FROM temp.h0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            rows.sort_unstable();
            got.push((rows, counts));
        }
        assert!(got[0].0.len() > 500, "{} rows", got[0].0.len());
        let longest: i64 = db.query_row("SELECT max(length(CAST(c0 AS BLOB))) FROM temp.n", [], |r| r.get(0)).unwrap();
        assert!(longest > 64, "a name past two chunks: {longest} bytes");
        assert_eq!(got[0].0, got[1].0, "the device's text tests are the CPU's");
        assert_eq!(got[0].1, got[1].1, "the device's counts per step are the CPU's");
    }
}
