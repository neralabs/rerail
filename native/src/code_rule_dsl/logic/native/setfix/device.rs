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
pub use crate::code_rule_dsl::logic::device::{Report};

/// The most rows a table is given before its stage has run once (a guess; an overflow runs it again exactly).
const GUESS_MAX: usize = 1 << 21;
/// The device's buffers share the Metal budget (`metal_buffer_limit`, 512 MB by default; decision 0126) with the
/// buffer pool and the other lanes. The device joins keep within about a quarter of it at the default (their sizes
/// below are those at 512 MB, scaled with the budget, `metal_share`):
/// the most bytes one binding table may take: past it the rule is refused at run time and runs on the CPU, named;
fn table_max() -> usize { rerail_accel::budget::metal_share(48 << 20) }
/// the most bytes one submission's tables may take: past it the rules after go into the next submission;
fn batch_max() -> usize { rerail_accel::budget::metal_share(64 << 20) }
/// The largest text arena a batch runs again with; past it, the stratum's texts are made on the host.
fn arena_max() -> usize { rerail_accel::budget::metal_share(64 << 20) }
/// the most bytes the relations kept on the device may take on a thread;
fn resident_max() -> usize { rerail_accel::budget::metal_share(80 << 20) }
/// the budget a submission leaves for its stages run again after an overflow: with less left, the rules after go
/// into the next submission.
fn headroom() -> usize { rerail_accel::budget::metal_share(48 << 20) }

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
    /// A stage that reads names (`str_cmp`, a cat compared): the texts of the columns that bound its slots and of its
    /// constants, on the device together.
    Names { cols: Vec<(Arc<InRel>, usize)>, consts: Vec<i64> },
    /// Text functions `ops`, in order, of the distinct symbols of column `col` of `rel`: their texts made on the
    /// device a SIMD group a symbol, 32 bytes at a time (`mj::Batch::text_map`, `replace`), kept with the relation;
    /// the stage copies a binding's into the arena.
    Mapped { rel: Arc<InRel>, col: usize, ops: Vec<TextOp> },
    /// A made text per row (`mj::ROWTEXT`: a cat, a regex's replacement, a text test of one), a SIMD group a row:
    /// over the names of `cols`' symbols and `consts` (as `Names`), with its automaton.
    RowText { cols: Vec<(Arc<InRel>, usize)>, consts: Vec<i64>, auto: RowAuto },
}

/// A `Kind::RowText` stage's automaton.
#[derive(Clone, Debug)]
enum RowAuto {
    None,
    /// A text test's table (`text::table`), keyed by its plans.
    Test { table: Arc<crate::code_rule_dsl::text::table::Table>, key: String },
    /// `replace_all` of regex `pattern` by `with` (symbols).
    Replace { pattern: i64, with: i64 },
}

/// Whether a stage makes texts into the arena (its counter word says when the arena was full).
fn makes_text(mode: u32) -> bool {
    mode == mj::ROWTEXT
}

/// A `ROWTEXT` stage of op `op` (`mj::ROW_*`, with a test's verdict wanted at bit 8) over operands `x` and `y` (a
/// symbol, a held cat's text, an arena text).
fn row_stage(first: usize, op: u32, x: Key, y: Key, auto: RowAuto, deferred: &HashMap<usize, (Key, Key, u32)>, origin: &[Option<(Arc<InRel>, usize)>], known: &HashMap<usize, i64>) -> Result<PStage, String> {
    let mut st = text_stage(mj::ROWTEXT, first, op, x, y, deferred, origin, known)?;
    let Kind::Names { cols, consts } = std::mem::replace(&mut st.kind, Kind::Pass) else { return Err("a made text's operands of no names".into()) };
    st.kind = Kind::RowText { cols, consts, auto };
    Ok(st)
}

/// A text function of constant arguments, made once per distinct symbol (`Kind::Mapped`).
#[derive(Clone, Debug, PartialEq)]
enum TextOp {
    /// `mj::TEXTFN_*` `f` of numbers `n0`, `n1` (`text_map`).
    Map { f: u32, n0: i64, n1: i64 },
    /// `replace_all` of a regex by a literal (symbols), or (`literal`) of the pattern's text itself.
    Replace { pattern: i64, with: i64, literal: bool },
}

/// The texts of `keys` (slots an input column bound, or constants), for a stage that reads names: the columns and
/// the constants, into `d`'s keys `0..` as slots or constants.
thread_local! {
    /// The step that first bound each slot of the rule being compiled (its name): what a refusal of a slot names.
    static BINDER: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The slots of the rule being compiled whose values are arena texts (`mj::ROWTEXT`'s).
    static ARENA: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Whether slot `x` of the rule being compiled holds an arena text (a text function's value).
fn is_arena(x: usize) -> bool {
    ARENA.with_borrow(|a| a.contains(&x))
}

/// A text function the device makes (`mj::TEXTFN_*`), its text arguments then its numbers.
fn text_function(name: &str) -> Option<(u32, usize, usize)> {
    Some(match name {
        "lower" => (mj::TEXTFN_LOWER, 1, 0),
        "sql_lower" => (mj::TEXTFN_ASCII_LOWER, 1, 0),
        "sql_upper" => (mj::TEXTFN_ASCII_UPPER, 1, 0),
        "head" => (mj::TEXTFN_HEAD, 1, 0),
        "dir" => (mj::TEXTFN_DIR, 1, 0),
        "rr_substr" => (mj::TEXTFN_SUBSTR, 1, 2),
        "sql_substr" => (mj::TEXTFN_SQL_SUBSTR, 1, 2),
        "slice" => (mj::TEXTFN_SLICE, 1, 2),
        "rr_to_string" => (mj::TEXTFN_TO_STRING, 0, 1),
        "sql_replace" => (mj::TEXTFN_REPLACE, 3, 0),
        _ => return None,
    })
}

fn text_keys(d: &mut mj::Stage, keys: &[Key], origin: &[Option<(Arc<InRel>, usize)>], known: &HashMap<usize, i64>, what: &str) -> Result<Kind, String> {
    let (mut cols, mut consts): (Vec<(Arc<InRel>, usize)>, Vec<i64>) = (Vec::new(), Vec::new());
    for (k, a) in keys.iter().enumerate() {
        // A slot a `let` filled with a constant is that constant.
        let a = match *a {
            Key::Slot(x) => known.get(&x).map_or(Key::Slot(x), |&c| Key::Const(c)),
            c => c,
        };
        match a {
            Key::Slot(x) if is_arena(x) => {
                // An arena text: its value says where its bytes are.
                d.key_kind[k] = mj::SLOT;
                d.key_slot[k] = x as u32;
            }
            Key::Slot(x) => {
                let Some(o) = origin.get(x).cloned().flatten() else {
                    let by = BINDER.with_borrow(|b| b.get(x).copied().unwrap_or("nothing"));
                    return Err(format!("{what} of a slot no input relation's column bound (bound by {by}) has no device form"));
                };
                if !cols.iter().any(|(r, c)| Arc::ptr_eq(r, &o.0) && *c == o.1) {
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
    Ok(Kind::Names { cols, consts })
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
pub(in crate::code_rule_dsl) struct Plan {
    stages: Vec<PStage>,
    head: Vec<Option<i64>>,
    head_slots: Vec<usize>,
    /// Per head column, whether its values are arena texts (a text function's), stored as symbols as the rows are.
    arena: Vec<bool>,
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
        Step::Func { name, .. } => name,
        Step::Doc { .. } => "document test",
        Step::DocOf { .. } => "docof",
        Step::Test { op, .. } => op,
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
    use rerail_compiler::block::{BlockOp, CmpOp, Rhs};
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
pub(in crate::code_rule_dsl) fn compile(j: &JoinRule, spec: &str) -> Result<Plan, String> {
    let mut bound = vec![false; j.nslots];
    // Where each slot's value came from: a column of an input relation (a text test's terms are its symbols).
    let mut origin: Vec<Option<(Arc<InRel>, usize)>> = vec![None; j.nslots];
    // Slots a `let` filled with a constant (a text stage reads the constant).
    let mut known: HashMap<usize, i64> = HashMap::new();
    BINDER.with_borrow_mut(|b| {
        b.clear();
        b.resize(j.nslots, "nothing");
        let mut seen = vec![false; j.nslots];
        for st in &j.steps {
            for x in 0..j.nslots {
                if !seen[x] && binds_slot(st, x) {
                    seen[x] = true;
                    b[x] = match st {
                        Step::Atom { src: Src::In(..), .. } => "an input relation's atom",
                        Step::Atom { .. } => "an own relation's atom",
                        other => step_name(other),
                    };
                }
            }
        }
    });
    ARENA.with_borrow_mut(Vec::clear);
    // Slots whose texts are text functions of an input column's symbols, made once per symbol: the column, the slot
    // that bound the symbol, and the functions.
    let mut mapped: HashMap<usize, (Arc<InRel>, usize, usize, Vec<TextOp>)> = HashMap::new();
    // `cat`s held back, by the slot they bind: each read only by an `eq` or `ne` after, which compares names.
    let mut deferred: HashMap<usize, (Key, Key, u32)> = HashMap::new();
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
            Step::Test { op, s, p, want, .. }
                if literal_test(op).is_some() && (matches!(p, Key::Slot(_)) || [s, p].iter().any(|k| matches!(k, Key::Slot(x) if deferred.contains_key(x) || is_arena(*x)))) =>
            {
                // A literal test whose pattern is bound (or a cat's text): the two texts compared on the device a
                // word at a time, pairwise, as `strings::test` does (a pattern holding the set separator refused).
                let op = literal_test(op).expect("a literal test") | mj::TEXT_SPLIT;
                stages.push(textop(i, op, *want, *p, *s, &deferred, &origin, &known)?);
                i += 1;
            }
            Step::Test { op, s: Key::Slot(x), matcher, want, .. } if is_arena(*x) || deferred.contains_key(x) => {
                // A text test by automaton of a made text (an arena text, a held cat's): its table walked per row, a
                // SIMD group a row, 32 bytes at a time (`ROWTEXT`).
                let Some(m) = matcher else { return Err(format!("the text test @{op} of a pattern that is not a constant has no device form")) };
                let table = crate::code_rule_dsl::text::table::table(&m.plans).map_err(|e| format!("the text test @{op} has no device form: {e}"))?;
                let key = m.plans.iter().map(|p| p.key.0.as_str()).collect::<Vec<_>>().join(",");
                stages.push(row_stage(i, mj::ROW_TEST | u32::from(*want) << 8, Key::Slot(*x), Key::Slot(*x), RowAuto::Test { table, key }, &deferred, &origin, &known)?);
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
                    if matches!(src, Key::Slot(x) if is_arena(x)) {
                        if doc {
                            return Err("a document of an arena text has no device form".into());
                        }
                        ARENA.with_borrow_mut(|a| a.push(out));
                    }
                    match (doc, src) {
                        (false, Key::Const(c)) => known.insert(out, c),
                        (false, Key::Slot(x)) => known.get(&x).copied().map(|c| known.insert(out, c)).flatten(),
                        _ => known.remove(&out),
                    };
                    n += 1;
                }
                d.nbinds = n as u32;
                stages.push(PStage { first: i, span: n, kind: Kind::Map, desc: d, npassed: 0 });
                i += n;
            }
            _ if joined(step).is_some() => {
                let (out, a, b, bind, form) = joined(step).expect("a joined text");
                // A text made of two names. Read only by comparisons (`eq`, `ne`, literal tests, `str_cmp`,
                // `strlen`), it is held: compared where it is read, from its parts. Read by the head or a text
                // function, it is made into the batch's arena (`TEXTFN_COPY`), as a text function's text is. Bound
                // already, it is that comparison here.
                let later: Vec<&Step> = j.steps[i + 1..].iter().filter(|s| mentions(s, out)).collect();
                let in_head = j.out.iter().any(|t| matches!(*t, Term::Slot(s) if s == out));
                let in_head_cat = j.out.iter().any(|t| matches!(*t, Term::Cat(x, y) if x == out || y == out));
                let made = [a, b].iter().any(|k| matches!(k, Key::Slot(s) if deferred.contains_key(s) || is_arena(*s)));
                // Read as its parts: comparisons, and what reads a text a SIMD group a row (`ROWTEXT`: a text test by
                // automaton, a regex's replacement, a cat).
                let compares = |s: &&Step| {
                    matches!(s, Step::Eq { .. } | Step::Ne { .. } | Step::Func { name: "str_cmp" | "rr_strlen", bind: true, .. })
                        || matches!(s, Step::Test { op, .. } if literal_test(op).is_some())
                        || matches!(s, Step::Test { p, matcher: Some(_), .. } if !matches!(p, Key::Slot(x) if *x == out))
                        || matches!(s, Step::Func { name: "replace_all" | "rx_replace", args, bind: true, .. } if arena_reader(s, out))
                        || joined(s).is_some_and(|(o, ..)| o != out)
                };
                if made {
                    // A cat of a made text (an arena text, a held cat's): made per row into the arena, a SIMD group a
                    // row (`ROWTEXT`).
                    if !bind {
                        return Err("a cat of a made text compared with a bound slot has no device form".into());
                    }
                    if in_head_cat {
                        return Err("cat of a made text in the head has no device form".into());
                    }
                    if let Some(s) = later.iter().find(|s| !arena_reader(s, out)) {
                        return Err(format!("a cat's text read by {} has no device form", step_name(s)));
                    }
                    let op = if form == mj::CAT_PLAIN { mj::ROW_CONCAT } else { mj::ROW_CAT };
                    let mut st = row_stage(i, op, a, b, RowAuto::None, &deferred, &origin, &known)?;
                    st.desc.bind_slot[0] = out as u32;
                    stages.push(st);
                    if out >= bound.len() {
                        bound.resize(out + 1, false);
                    }
                    bound[out] = true;
                    ARENA.with_borrow_mut(|x| x.push(out));
                } else if bind && !in_head && !in_head_cat && later.iter().all(compares) {
                    deferred.insert(out, (a, b, form));
                    stages.push(PStage { first: i, span: 1, kind: Kind::Pass, desc: mj::Stage::default(), npassed: 0 });
                } else if bind {
                    if in_head_cat {
                        return Err("cat of a made text has no device form".into());
                    }
                    if let Some(s) = later.iter().find(|s| !arena_reader(s, out)) {
                        return Err(format!("a cat's text read by {} has no device form", step_name(s)));
                    }
                    stages.push(copy_stage(i, out, a, b, form, &origin, &known)?);
                    if out >= bound.len() {
                        bound.resize(out + 1, false);
                    }
                    bound[out] = true;
                    ARENA.with_borrow_mut(|x| x.push(out));
                } else {
                    let mut held = HashMap::new();
                    held.insert(usize::MAX, (a, b, form));
                    stages.push(textop(i, mj::TEXT_EQ, true, Key::Slot(usize::MAX), Key::Slot(out), &held, &origin, &known)?);
                }
                i += 1;
            }
            Step::Eq { a, b } | Step::Ne { a, b } if [a, b].iter().any(|k| matches!(k, Key::Slot(s) if deferred.contains_key(s) || is_arena(*s))) => {
                stages.push(textop(i, mj::TEXT_EQ, matches!(step, Step::Eq { .. }), *a, *b, &deferred, &origin, &known)?);
                i += 1;
            }
            Step::Lt { .. } | Step::Eq { .. } | Step::Ne { .. } | Step::Doc { .. } | Step::DocOf { bind: false, .. } | Step::In { .. } => {
                // The filters over the bindings: the row program `rowcode` makes for an atom's, with every slot
                // bound so far a column of its own.
                let slots: Vec<(usize, usize)> = (0..bound.len()).filter(|&s| bound[s] && !deferred.contains_key(&s)).map(|s| (s, s)).collect();
                // Up to the first step that reads a held cat's text (its own stage).
                let stop = (i..j.steps.len())
                    .find(|&k| deferred.keys().copied().chain(ARENA.with_borrow(Clone::clone)).any(|x| mentions(&j.steps[k], x)))
                    .unwrap_or(j.steps.len());
                let Some(f) = rowcode::fused(&slots, &j.steps[i..stop]) else { return Err(format!("{} has no device form", step_name(step))) };
                let mut d = mj::Stage { mode: mj::FILTER, ..mj::Stage::default() };
                program(&mut d, &f, mj::LEFT)?;
                binds(&mut d, &f, true);
                for &(s, _) in &f.binds { bound[s] = true; }
                stages.push(PStage { first: i, span: f.steps, kind: Kind::Filter, desc: d, npassed: f.steps });
                i += f.steps;
            }
            Step::Func { name: name @ ("str_cmp" | "rr_strlen"), args, out, bind: true, .. } => {
                // A function of names: the order of two (compared a word of eight bytes at a time) or one's
                // characters, their texts those of the columns that bound the slots (and the constants').
                let f = if *name == "str_cmp" { mj::NAMES_CMP } else { mj::NAMES_LEN };
                let y = args.get(1).copied().unwrap_or(args[0]);
                let mut st = text_stage(mj::STRCMP, i, f, args[0], y, &deferred, &origin, &known)?;
                st.desc.bind_slot[0] = *out as u32;
                if *out >= bound.len() {
                    bound.resize(*out + 1, false);
                }
                bound[*out] = true;
                stages.push(st);
                i += 1;
            }
            Step::Func { name, args, out, bind: true, .. } if per_symbol(name, args, &known, &origin, &mapped, &deferred).is_some() => {
                // A text function of constant arguments of an input column's symbol (or of such a function's text):
                // made once per distinct symbol of the column, 32 bytes at a time, the binding's copied into the arena.
                let (op, x) = per_symbol(name, args, &known, &origin, &mapped, &deferred).expect("a text function per symbol");
                let later: Vec<&Step> = j.steps[i + 1..].iter().filter(|s| mentions(s, *out)).collect();
                if let Some(s) = later.iter().find(|s| !arena_reader(s, *out)) {
                    return Err(format!("the function {name}'s text read by {} has no device form", step_name(s)));
                }
                if j.out.iter().any(|t| matches!(*t, Term::Cat(a, b) if a == *out || b == *out)) {
                    return Err(format!("cat of the function {name}'s text has no device form"));
                }
                let (rel, col, src, mut ops) = match mapped.get(&x) {
                    Some(m) => m.clone(),
                    None => {
                        let (rel, col) = origin[x].clone().expect("an input column's symbol");
                        (rel, col, x, Vec::new())
                    }
                };
                ops.extend(op);
                let mut d = mj::Stage { mode: mj::ROWTEXT, ..mj::Stage::default() };
                d.key_kind[0] = mj::SLOT;
                d.key_slot[0] = src as u32;
                d.agg = [mj::ROW_MAP | mj::TEXTFN_COPY << 16, mj::OPND_SYM];
                d.bind_slot[0] = *out as u32;
                if *out >= bound.len() {
                    bound.resize(*out + 1, false);
                }
                bound[*out] = true;
                ARENA.with_borrow_mut(|a| a.push(*out));
                mapped.insert(*out, (rel.clone(), col, src, ops.clone()));
                stages.push(PStage { first: i, span: 1, kind: Kind::Mapped { rel, col, ops }, desc: d, npassed: 0 });
                i += 1;
            }
            Step::Func { name: name @ ("replace_all" | "rx_replace"), args, out, bind: true, .. }
                if args.len() == 3 && matches!(args[0], Key::Slot(x) if is_arena(x) || deferred.contains_key(&x)) =>
            {
                // A regex's replacement in a made text (an arena text, a held cat's): per row, a SIMD group a row, 32
                // bytes at a time (`ROWTEXT`), into the arena.
                let konst = |k: Key| match k {
                    Key::Const(c) => Some(c),
                    Key::Slot(x) => known.get(&x).copied(),
                };
                let (Some(pattern), Some(with)) = (konst(args[1]), konst(args[2])) else {
                    return Err(format!("the function {name} of a regex or replacement that is not a constant has no device form"));
                };
                let later: Vec<&Step> = j.steps[i + 1..].iter().filter(|s| mentions(s, *out)).collect();
                if let Some(s) = later.iter().find(|s| !arena_reader(s, *out)) {
                    return Err(format!("the function {name}'s text read by {} has no device form", step_name(s)));
                }
                if j.out.iter().any(|t| matches!(*t, Term::Cat(a, b) if a == *out || b == *out)) {
                    return Err(format!("cat of the function {name}'s text has no device form"));
                }
                let mut st = row_stage(i, mj::ROW_REPLACE, args[0], Key::Const(with), RowAuto::Replace { pattern, with }, &deferred, &origin, &known)?;
                st.desc.bind_slot[0] = *out as u32;
                stages.push(st);
                if *out >= bound.len() {
                    bound.resize(*out + 1, false);
                }
                bound[*out] = true;
                ARENA.with_borrow_mut(|a| a.push(*out));
                i += 1;
            }
            Step::Func { name: name @ ("replace_all" | "rx_replace"), .. } => {
                return Err(format!("the function {name} of a text no input relation's column bound, or of a regex or replacement not a constant, has no device form (it is made per distinct name)"));
            }
            Step::Func { name, args, out, bind: true, .. } if text_function(name).is_some() => {
                // A text function: its text made into the batch's arena (or a range of its argument's), its value
                // where the text is. Read by text tests, `str_cmp`, `strlen`, other text functions and the head (as a
                // symbol), as a held cat's text is.
                let (f, ntexts, nnums) = text_function(name).expect("a text function");
                if args.len() != ntexts + nnums {
                    return Err(format!("the function {name} of {} arguments", args.len()));
                }
                let later: Vec<&Step> = j.steps[i + 1..].iter().filter(|s| mentions(s, *out)).collect();
                if let Some(s) = later.iter().find(|s| !arena_reader(s, *out)) {
                    return Err(format!("the function {name}'s text read by {} has no device form", step_name(s)));
                }
                if j.out.iter().any(|t| matches!(*t, Term::Cat(x, y) if x == *out || y == *out)) {
                    return Err(format!("cat of the function {name}'s text has no device form"));
                }
                let mut d = mj::Stage { mode: mj::ROWTEXT, ..mj::Stage::default() };
                let mut xk = mj::OPND_SYM;
                let kind = if ntexts > 0 {
                    // X (keys 0, 1), Y and Z (keys 2 and 4).
                    let mut keys = vec![args[0], args[0]];
                    if ntexts == 3 {
                        keys.extend([args[1], args[1], args[2]]);
                    }
                    if let Key::Slot(x) = args[0] {
                        if is_arena(x) {
                            xk = mj::OPND_ARENA;
                        } else if let Some(&(pa, pb, form)) = deferred.get(&x) {
                            // A held cat's text: read from its parts.
                            keys[0] = pa;
                            keys[1] = pb;
                            xk = if form == mj::CAT_PLAIN { mj::OPND_PLAIN } else { mj::OPND_DOT };
                        }
                    }
                    if args[1..ntexts].iter().any(|k| matches!(k, Key::Slot(x) if is_arena(*x) || deferred.contains_key(x))) {
                        return Err(format!("the function {name}'s pattern or replacement made by the rule has no device form"));
                    }
                    text_keys(&mut d, &keys, &origin, &known, &format!("the function {name}"))?
                } else {
                    Kind::Names { cols: Vec::new(), consts: Vec::new() }
                };
                // The numbers (keys 5 and 6).
                for (n, a) in args[ntexts..].iter().enumerate() {
                    let k = 5 + n;
                    let a = match *a {
                        Key::Slot(x) => known.get(&x).map_or(Key::Slot(x), |&c| Key::Const(c)),
                        c => c,
                    };
                    match a {
                        Key::Slot(x) if is_arena(x) || deferred.contains_key(&x) => return Err(format!("the function {name} of a text where a number is")),
                        Key::Slot(x) => {
                            d.key_kind[k] = mj::SLOT;
                            d.key_slot[k] = x as u32;
                        }
                        Key::Const(c) => {
                            d.key_kind[k] = mj::CONST;
                            d.key_const[k] = c;
                        }
                    }
                }
                d.agg = [mj::ROW_MAP | f << 16, xk];
                d.bind_slot[0] = *out as u32;
                if *out >= bound.len() {
                    bound.resize(*out + 1, false);
                }
                bound[*out] = true;
                ARENA.with_borrow_mut(|a| a.push(*out));
                stages.push(PStage { first: i, span: 1, kind, desc: d, npassed: 0 });
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
    let mut arena = Vec::new();
    let mut nslots = j.nslots;
    for t in &j.out {
        match *t {
            Term::Slot(s) => {
                head.push(None);
                head_slots.push(s);
                arena.push(is_arena(s));
            }
            Term::Const(c) => {
                head.push(Some(c));
                head_slots.push(0);
                arena.push(false);
            }
            Term::Cat(x, y) => {
                // `cat` in the head: made into the arena, in a slot of its own after the steps (a stage that counts
                // no step).
                let mut st = copy_stage(j.steps.len(), nslots, Key::Slot(x), Key::Slot(y), mj::CAT_DOT, &origin, &known)?;
                st.span = 0;
                stages.push(st);
                head.push(None);
                head_slots.push(nslots);
                arena.push(true);
                nslots += 1;
            }
        }
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = rustc_hash::FxHasher::default();
        spec.hash(&mut h);
        h.finish()
    };
    Ok(Plan { stages, head, head_slots, arena, nslots, nsteps: j.steps.len(), key })
}

/// Whether `step` binds slot `x`.
fn binds_slot(step: &Step, x: usize) -> bool {
    match step {
        Step::Atom { binds, neg: false, .. } => binds.iter().any(|&(_, s)| s == x),
        Step::Scalar { out, bind, .. } | Step::StrJoin { out, bind, .. } => *bind && *out == x,
        Step::Cat { out, bind, .. } | Step::Func { out, bind, .. } | Step::DocOf { out, bind, .. } => *bind && *out == x,
        Step::Let { a, .. } => *a == x,
        Step::Split { a, b, .. } => *a == Some(x) || *b == Some(x),
        _ => false,
    }
}

/// Whether `step` reads slot `x`.
fn mentions(step: &Step, x: usize) -> bool {
    let k = |key: &Key| matches!(key, Key::Slot(s) if *s == x);
    match step {
        Step::Unused { .. } => false,
        Step::Atom { keys, .. } | Step::Scalar { keys, .. } | Step::StrJoin { keys, .. } => keys.iter().any(|(_, key)| k(key)),
        Step::Cat { a, b, .. } => k(a) || k(b),
        Step::Split { whole, ka, kb, .. } => k(whole) || k(ka) || k(kb),
        Step::Eq { a, b } | Step::Ne { a, b } | Step::Lt { a, b, .. } | Step::Doc { a, b, .. } => k(a) || k(b),
        Step::Let { b, .. } => k(b),
        Step::Func { args, .. } => args.iter().any(k),
        Step::DocOf { out, n, bind } => k(n) || (!*bind && *out == x),
        Step::Test { s, p, .. } => k(s) || k(p),
        Step::In { a, .. } => k(a),
    }
}

/// The literal text tests the device runs of any texts (`mj::TEXTOP`): `equals`, `prefix`, `suffix`.
fn literal_test(op: &str) -> Option<u32> {
    match op {
        "equals" => Some(mj::TEXT_EQ),
        "prefix" => Some(mj::TEXT_PREFIX),
        "suffix" => Some(mj::TEXT_SUFFIX),
        _ => None,
    }
}

/// A `TEXTOP` stage: test `op` (`mj::TEXT_*`, with `mj::TEXT_SPLIT` for a text test) of pattern `x` and subject `y`,
/// each a symbol or a held cat's text (`deferred`).
fn textop(first: usize, op: u32, want: bool, x: Key, y: Key, deferred: &HashMap<usize, (Key, Key, u32)>, origin: &[Option<(Arc<InRel>, usize)>], known: &HashMap<usize, i64>) -> Result<PStage, String> {
    text_stage(mj::TEXTOP, first, u32::from(want) | op << 1, x, y, deferred, origin, known)
}

/// A stage of mode `mode` over operands `x` and `y` (keys 0, 1 and 2, 3), each a symbol or a held cat's text, its
/// `agg[0]` given.
fn text_stage(mode: u32, first: usize, agg0: u32, x: Key, y: Key, deferred: &HashMap<usize, (Key, Key, u32)>, origin: &[Option<(Arc<InRel>, usize)>], known: &HashMap<usize, i64>) -> Result<PStage, String> {
    let operand = |k: Key| match k {
        Key::Slot(s) if deferred.contains_key(&s) => {
            let (a, b, form) = deferred[&s];
            (a, b, if form == mj::CAT_PLAIN { mj::OPND_PLAIN } else { mj::OPND_DOT })
        }
        Key::Slot(s) if is_arena(s) => (k, k, mj::OPND_ARENA),
        k => (k, k, mj::OPND_SYM),
    };
    let ((xa, xb, xk), (ya, yb, yk)) = (operand(x), operand(y));
    let mut d = mj::Stage { mode, ..mj::Stage::default() };
    let kind = text_keys(&mut d, &[xa, xb, ya, yb], origin, known, if mode == mj::TEXTOP { "a text test" } else { "a function of names" })?;
    d.agg = [agg0, xk | yk << 4];
    Ok(PStage { first, span: 1, kind, desc: d, npassed: 0 })
}

/// A text made of two names, unless it is bound already: its slot, the two parts, whether it binds, and its form
/// (`mj::CAT_DOT`: `cat`'s, the second alone when the first is empty, else joined by "."; `mj::CAT_PLAIN`:
/// `str_concat`'s, the two together).
fn joined(step: &Step) -> Option<(usize, Key, Key, bool, u32)> {
    match step {
        Step::Cat { out, a, b, bind } => Some((*out, *a, *b, *bind, mj::CAT_DOT)),
        Step::Func { name: "str_concat", args, out, bind, .. } if args.len() == 2 => Some((*out, args[0], args[1], *bind, mj::CAT_PLAIN)),
        _ => None,
    }
}

/// Whether `step` reads slot `x`'s arena text as the device can: a comparison, a literal test of it (not as a
/// pattern), `str_cmp`, `strlen`, a `let`, or a text function's text argument.
fn arena_reader(step: &Step, x: usize) -> bool {
    let me = |k: &Key| matches!(k, Key::Slot(s) if *s == x);
    match step {
        Step::Eq { .. } | Step::Ne { .. } | Step::Func { name: "str_cmp" | "rr_strlen", bind: true, .. } | Step::Let { .. } => true,
        Step::Test { op, p, matcher, .. } => (literal_test(op).is_some() || matcher.is_some()) && !me(p),
        Step::Cat { bind: true, .. } | Step::Func { name: "str_concat", bind: true, .. } => true,
        Step::Func { name: "replace_all" | "rx_replace", args, bind: true, .. } => args.len() == 3 && me(&args[0]) && !args[1..].iter().any(me),
        Step::Func { name, args, bind: true, .. } => text_function(name).is_some_and(|(_, nt, _)| nt > 0) && me(&args[0]) && !args[1..].iter().any(me),
        _ => false,
    }
}

/// A text function step made once per distinct symbol (`Kind::Mapped`): its function (none: the identity, as
/// `sql_replace` of an empty pattern) and the slot of its text, when that text is an input column's symbol (or a
/// mapped text) and its other arguments are constants.
fn per_symbol(name: &str, args: &[Key], known: &HashMap<usize, i64>, origin: &[Option<(Arc<InRel>, usize)>],
              mapped: &HashMap<usize, (Arc<InRel>, usize, usize, Vec<TextOp>)>, deferred: &HashMap<usize, (Key, Key, u32)>) -> Option<(Option<TextOp>, usize)> {
    let konst = |k: &Key| match *k {
        Key::Const(c) => Some(c),
        Key::Slot(x) => known.get(&x).copied(),
    };
    let Some(&Key::Slot(x)) = args.first() else { return None };
    let column = mapped.contains_key(&x) || (!is_arena(x) && !deferred.contains_key(&x) && origin.get(x).is_some_and(Option::is_some));
    if !column {
        return None;
    }
    let consts: Option<Vec<i64>> = args[1..].iter().map(konst).collect();
    let consts = consts?;
    let op = match name {
        "replace_all" | "rx_replace" if consts.len() == 2 => TextOp::Replace { pattern: consts[0], with: consts[1], literal: false },
        "sql_replace" if consts.len() == 2 => TextOp::Replace { pattern: consts[0], with: consts[1], literal: true },
        _ => {
            let (f, nt, nn) = text_function(name)?;
            if nt != 1 || f == mj::TEXTFN_TO_STRING || consts.len() != nn {
                return None;
            }
            TextOp::Map { f, n0: consts.first().copied().unwrap_or(0), n1: consts.get(1).copied().unwrap_or(0) }
        }
    };
    Some((Some(op), x))
}

/// A `TEXTFN_COPY` stage: the text of `a` and `b` by `form` (`cat`'s or `str_concat`'s) made into the arena, its
/// value into slot `out`.
fn copy_stage(first: usize, out: usize, a: Key, b: Key, form: u32, origin: &[Option<(Arc<InRel>, usize)>], known: &HashMap<usize, i64>) -> Result<PStage, String> {
    let mut d = mj::Stage { mode: mj::ROWTEXT, ..mj::Stage::default() };
    let kind = text_keys(&mut d, &[a, b], origin, known, "cat")?;
    d.agg = [mj::ROW_MAP | mj::TEXTFN_COPY << 16, if form == mj::CAT_PLAIN { mj::OPND_PLAIN } else { mj::OPND_DOT }];
    d.bind_slot[0] = out as u32;
    Ok(PStage { first, span: 1, kind, desc: d, npassed: 0 })
}

/// A relation kept on the device: its rows, and its sorts by key columns.
struct Entry {
    rel: Weak<InRel>,
    dev: Rc<mj::DevRel>,
    indexes: Vec<(Vec<usize>, Rc<mj::DevIndex>)>,
    /// Per column a text test read, its distinct symbols' names; per (column, test), the test's verdicts.
    texts: Vec<(usize, Rc<mj::DevText>)>,
    verdicts: Vec<((usize, String), Rc<mj::DevRel>)>,
    /// Per (column, text functions), the column's symbols' texts made (`mapped_text`), with `lower`'s check.
    mapped: Vec<((usize, String), (Rc<mj::DevText>, Option<Rc<mj::Words>>))>,
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
        if self.bytes() + room > resident_max() {
            self.unions.clear();
        }
        while self.bytes() + room > resident_max() && !self.entries.is_empty() {
            let at = self.entries.iter().enumerate().min_by_key(|(_, e)| e.used).map(|(k, _)| k).expect("an entry");
            self.entries.swap_remove(at);
        }
        self.bytes = self.bytes();
    }

    fn bytes(&self) -> usize {
        self.entries.iter().map(|e| e.bytes).sum::<usize>() + self.unions.iter().map(|u| u.2.bytes()).sum::<usize>()
    }
}

/// The report of the last stratum the device ran or refused on this thread (taken).
pub(in crate::code_rule_dsl) fn take_report() -> Option<Report> {
    LAST.with_borrow_mut(Option::take)
}

/// Notes that the device refused the stratum being opened or run: it runs on the CPU, and the profile names why.
pub(in crate::code_rule_dsl) fn refuse(why: String) {
    LAST.with_borrow_mut(|l| *l = Some(Report { refused: Some(why), ..Report::default() }));
}

/// A rule's state in a batch: its tables by totals slot, their capacities, its words.
struct Run<'p> {
    /// Its own copy: a run waiting for its wave outlives the borrow of the component's plans.
    plan: Rc<Plan>,
    _plan: std::marker::PhantomData<&'p Plan>,
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
    /// The text arena its text functions made their texts into (its batch's, shared with the batch's other jobs).
    arena: Option<Rc<mj::Arena>>,
    /// Words its texts made per symbol set when `lower` met a name past ASCII (`mapped_text`).
    checks: Vec<Rc<mj::Words>>,
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
                if mj::device_bytes_left() < 8 * k * n + headroom() {
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
                res.entries.push(Entry { rel: weak, dev: Rc::new(dev), indexes: Vec::new(), texts: Vec::new(), verdicts: Vec::new(), mapped: Vec::new(), bytes, used: clock });
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

/// The names of column `col`'s distinct symbols on the device, kept with the relation's entry `e` (copied once).
fn column_text(e: &mut Entry, rel: &InRel, col: usize, rep: &mut Report, names: &Interner) -> Result<Rc<mj::DevText>, String> {
    if let Some((_, t)) = e.texts.iter().find(|(c, _)| *c == col) {
        return Ok(t.clone());
    }
    let c0 = thread_cpu();
    let ids = column_symbols(rel, col);
    let t = Rc::new(mj::DevText::upload(&ids, |v, out| out.extend_from_slice(names.name(v).as_bytes())).map_err(|e| format!("{e:?}: the names of {} symbols", ids.len()))?);
    rep.uploads += 1;
    rep.upload_bytes += t.bytes() as u64;
    rep.upload_cpu += thread_cpu() - c0;
    e.bytes += t.bytes();
    e.texts.push((col, t.clone()));
    Ok(t)
}

/// Regex `regex`'s automata for its replacements on the device (`text::table::Replacer`), and their states; kept while
/// the thread runs (a command buffer may read them). Refused for a regex the device has no automata for.
fn automata(regex: &str) -> Result<(Rc<mj::DevBytes>, u32), String> {
    thread_local! {
        static AUTOMATA: RefCell<HashMap<String, (Rc<mj::DevBytes>, u32)>> = RefCell::new(HashMap::new());
    }
    if let Some(a) = AUTOMATA.with_borrow(|a| a.get(regex).cloned()) {
        return Ok(a);
    }
    let r = crate::code_rule_dsl::text::table::replacer(regex).map_err(|e| format!("replace_all has no device form: {e}"))?;
    let a = (Rc::new(mj::DevBytes::upload(&r.bytes).map_err(|e| format!("{e:?}"))?), r.states);
    AUTOMATA.with_borrow_mut(|m| m.insert(regex.to_owned(), a.clone()));
    Ok(a)
}

/// A `ROWTEXT` stage's automaton on the device and `Batch::row_stage`'s numbers for it ([its scratch's states, a
/// test table's states, classes, start]); a test's tables kept while the thread runs.
fn row_automaton(auto: &RowAuto, names: &Interner) -> Result<Option<(Rc<mj::DevBytes>, [u32; 4])>, String> {
    thread_local! {
        static TESTS: RefCell<HashMap<String, Rc<mj::DevBytes>>> = RefCell::new(HashMap::new());
    }
    Ok(match auto {
        RowAuto::None => None,
        RowAuto::Replace { pattern, with } => {
            if names.name(*with).contains('$') {
                return Err("replace_all with a replacement naming a group ($) has no device form".into());
            }
            let (a, states) = automata(&names.name(*pattern))?;
            Some((a, [states, 0, 0, 0]))
        }
        RowAuto::Test { table, key } => {
            let b = match TESTS.with_borrow(|t| t.get(key).cloned()) {
                Some(b) => b,
                None => {
                    let b = Rc::new(mj::DevBytes::upload(&table.bytes).map_err(|e| format!("{e:?}"))?);
                    TESTS.with_borrow_mut(|t| t.insert(key.clone(), b.clone()));
                    b
                }
            };
            Some((b, [table.states, table.states, table.classes, table.start]))
        }
    })
}

/// Text functions `ops` of the distinct symbols of column `col` of `rel`, in order: their texts as a text buffer by
/// symbol (`DevText`'s), made on the device (`Batch::text_map`, `Batch::replace`: a SIMD group a symbol, 32 bytes at a
/// time) the first time a stage asks, and kept with the relation; with the word `lower` sets past ASCII (the run's
/// check). Refused (the stratum runs on the CPU) for a regex the device has no automata for, or a replacement naming a
/// group (`$`).
fn mapped_text(rel: &Arc<InRel>, col: usize, ops: &[TextOp], batch: &mut mj::Batch, rep: &mut Report, names: &Interner)
    -> Result<(Rc<mj::DevRel>, Option<Rc<mj::Words>>), String> {
    // The functions with their texts, as the key they are kept by.
    let mut steps = Vec::new();
    for op in ops {
        steps.push(match *op {
            TextOp::Map { f, n0, n1 } => (format!("{f}({n0},{n1})"), None),
            TextOp::Replace { pattern, with, literal } => {
                let (p, w) = (names.name(pattern), names.name(with));
                if !literal && w.contains('$') {
                    return Err("replace_all with a replacement naming a group ($) has no device form".into());
                }
                if literal && p.is_empty() {
                    // SQLite's replace of an empty pattern: the text itself.
                    continue;
                }
                let regex = if literal { regex_syntax::escape(&p) } else { p };
                let automata = automata(&regex)?;
                (format!("rx({regex:?},{w:?})"), Some((automata, w)))
            }
        });
    }
    resident(rel, &[], batch, rep)?;
    RESIDENT.with_borrow_mut(|res| {
        let weak = Arc::downgrade(rel);
        let e = res.entries.iter_mut().find(|e| Weak::ptr_eq(&e.rel, &weak)).expect("the relation made resident");
        let key = (col, steps.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";"));
        if let Some((_, (t, flag))) = e.mapped.iter().find(|(k, _)| *k == key) {
            rep.sort_hits += 1;
            return Ok((t.rel(), flag.clone()));
        }
        let mut text = column_text(e, rel, col, rep, names)?;
        let mut flag = None;
        for ((what, rx), op) in steps.iter().zip(ops.iter().filter(|op| !matches!(op, TextOp::Replace { pattern, literal: true, .. } if names.name(*pattern).is_empty()))) {
            let next = match (rx, op) {
                (Some(((automata, states), with)), _) => batch.replace(&text, automata, *states, with.as_bytes()),
                (None, TextOp::Map { f, n0, n1 }) => {
                    let w = flag.get_or_insert_with(|| Rc::new(mj::Words::zeroed(1).expect("a word")));
                    batch.text_map(&text, *f, *n0, *n1, w)
                }
                (None, TextOp::Replace { .. }) => unreachable!("a replacement has its automata"),
            };
            text = Rc::new(next.map_err(|e| format!("{e:?}: {what} over {} names", text.terms))?);
            rep.sorts += 1;
            rep.per_symbol += 1;
            rep.per_symbol_names += text.terms as u64;
        }
        e.bytes += text.bytes();
        let flag = flag.filter(|_| ops.iter().any(|op| matches!(op, TextOp::Map { f: mj::TEXTFN_LOWER, .. })));
        e.mapped.push((key, (text.clone(), flag.clone())));
        Ok((text.rel(), flag))
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
        let text = column_text(e, rel, col, rep, names)?;
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
    fn new(plan: &Plan) -> Result<Run<'p>, String> {
        let mut io = Vec::new();
        let mut slot = 0;
        let mut counter = Vec::new();
        let mut ncounters = 0;
        for s in &plan.stages {
            match s.kind {
                Kind::Pass | Kind::Map => io.push((slot, slot)),
                Kind::Filter | Kind::Atom { .. } | Kind::Own { .. } | Kind::Text { .. } | Kind::Names { .. } | Kind::Mapped { .. } | Kind::RowText { .. } => {
                    io.push((slot, slot + 1));
                    slot += 1;
                }
            }
            if s.desc.mode == mj::AGG || makes_text(s.desc.mode) {
                // Its word: set when a sum passed 64 bits, or a made text's part is past what a made value holds.
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
        Ok(Run { plan: Rc::new(plan.clone()), _plan: std::marker::PhantomData, io, tables, caps, totals, counters, counter, rels: vec![None; plan.stages.len()], uniq: None, uniq_cols, arena: None, checks: Vec::new() })
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
                Kind::Filter | Kind::Atom { .. } | Kind::Own { .. } | Kind::Text { .. } | Kind::Names { .. } | Kind::Mapped { .. } | Kind::RowText { .. } => {
                    match (&s.kind, &self.rels[k]) {
                        (Kind::Names { cols, consts }, None) => {
                            let t = texts(cols, consts, rep, names)?;
                            if s.desc.mode == mj::TEXTOP && s.desc.agg[0] & mj::TEXT_SPLIT != 0 && t.sep {
                                return Err("a text test of a name holding the set separator (a set of patterns) has no device form".into());
                            }
                            self.rels[k] = Some((t.rel(), None));
                        }
                        (Kind::Atom { rel, cols }, None) => self.rels[k] = Some(resident(rel, cols, batch, rep)?),
                        (Kind::Text { rel, col, table, key }, None) => self.rels[k] = Some((verdicts(rel, *col, table, key, batch, rep, names)?, None)),
                        (Kind::RowText { cols, consts, .. }, None) => self.rels[k] = Some((texts(cols, consts, rep, names)?.rel(), None)),
                        (Kind::Mapped { rel, col, ops }, None) => {
                            let (t, flag) = mapped_text(rel, *col, ops, batch, rep, names)?;
                            self.checks.extend(flag);
                            self.rels[k] = Some((t, None));
                        }
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
                    if 8 * self.plan.nslots.max(1) * cap > table_max() {
                        return Err(format!("a binding table of {cap} rows past the device's budget"));
                    }
                    let out = mj::Table::new(self.plan.nslots, cap).map_err(|e| format!("{e:?}: a binding table of {cap} rows, {} slots", self.plan.nslots))?;
                    if let Some(c) = self.counter[k] {
                        for w in c..c + 1 + if s.desc.mode == mj::AGG || makes_text(s.desc.mode) { 0 } else { s.npassed } {
                            self.counters.set(w, 0);
                        }
                    }
                    if makes_text(s.desc.mode) && k == self.first_textfn().unwrap_or(usize::MAX) {
                        // Its texts, and those of the text functions after it, go into this batch's arena.
                        self.arena = Some(batch.arena().map_err(|e| format!("{e:?}"))?);
                    }
                    let left = self.tables[i].as_ref().expect("the stage's input");
                    if s.desc.mode == mj::ROWTEXT {
                        let auto = match &s.kind {
                            Kind::RowText { auto, .. } => row_automaton(auto, names)?,
                            _ => None,
                        };
                        let names_rel = right.map(|r| r.0).ok_or("a made text's stage with no names")?;
                        batch.row_stage(s.desc, left, names_rel, auto.as_ref().map(|(b, p)| (&**b, *p)), i, o, &self.totals, &self.counters, self.counter[k], &out)
                    } else {
                        batch.stage(s.desc, left, right, i, o, &self.totals, &self.counters, self.counter[k], &out)
                    }
                    .map_err(|e| format!("{e:?}: a stage's space for {cap_in} rows"))?;
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

    /// Its first text function's stage: a run that runs again in a new batch runs again from it (or before), its
    /// texts all in the new batch's arena.
    fn first_textfn(&self) -> Option<usize> {
        self.plan.stages.iter().position(|s| makes_text(s.desc.mode))
    }

    /// Whether a text function's text found the arena full (`mj::TEXTFN_FULL`).
    fn arena_full(&self) -> bool {
        self.plan.stages.iter().zip(&self.counter).any(|(s, c)| makes_text(s.desc.mode) && c.is_some_and(|c| self.counters.get(c) & mj::TEXTFN_FULL != 0))
    }

    /// Why its rows are not the CPU's, once it ran: an aggregate's sum past 64 bits (the CPU's fold holds 128).
    fn failed(&self) -> Option<String> {
        let set = |mode| self.plan.stages.iter().zip(&self.counter).any(|(s, c)| s.desc.mode == mode && c.is_some_and(|c| self.counters.get(c) != 0));
        if set(mj::AGG) {
            return Some("an aggregate's sum past 64 bits".to_owned());
        }
        if self.checks.iter().any(|w| w.get(0) != 0)
            || self.plan.stages.iter().zip(&self.counter).any(|(s, c)| makes_text(s.desc.mode) && c.is_some_and(|c| self.counters.get(c) & mj::TEXTFN_HOST != 0))
        {
            return Some("lower of a name past ASCII (Unicode's mapping is the host's)".to_owned());
        }
        if self.arena_full() {
            return Some(format!("its texts past the device's text arena of {} bytes", arena_max()));
        }
        None
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
/// as few submissions as their tables' space allows (`batch_max`): one, for most strata.
pub(in crate::code_rule_dsl) fn run(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
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

thread_local! {
    /// The level's open command buffer (decision 0113 §3's wave): the strata placed on the device encode their joins
    /// into it and wait for the wave, so a level's small strata go out in one commit and one wait. Its number, and
    /// whether it is open.
    static WAVE: RefCell<Option<(u64, mj::Batch)>> = const { RefCell::new(None) };
    static WAVES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The waves flushed, by number: their completion (or why they failed), and the device's seconds not yet
    /// counted by a stratum.
    static FLUSHED: RefCell<HashMap<u64, Result<Option<mj::Done>, String>>> = RefCell::new(HashMap::new());
    /// Strata waiting for their wave, by ticket: their encoded runs, their wave, their report so far.
    static SUSPENDED: RefCell<HashMap<u64, (Vec<Run<'static>>, u64, Report)>> = RefCell::new(HashMap::new());
    static TICKETS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// A stratum's joins encoded into the level's open command buffer, to wait for the wave (`lanes::wave::flush`
/// commits it once, for every stratum in it): its ticket for `resume`. `None` when they do not all fit one
/// submission's space (the stratum then runs on its own, `run`).
pub(in crate::code_rule_dsl) fn submit(rules: &[(&Plan, &JoinRule, usize)], names: &Interner) -> Result<Option<u64>, String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "device joins (to the wave)".into());
    // A wave holds its strata's buffers to the level's end: with the budget's headroom gone (the wave's own leases
    // count, `device_bytes_left`), this stratum runs in a submission of its own, whose buffers go back when it ends.
    if mj::device_bytes_left() < headroom() {
        crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "strata run apart: the wave's budget was spent", 1);
        return Ok(None);
    }
    let c0 = thread_cpu();
    let mut rep = Report { rules: rules.len() as u32, stages: rules.iter().map(|r| r.0.stages.len() as u32).sum(), ..Report::default() };
    let opened = WAVE.with_borrow(Option::is_none);
    if opened {
        let n = WAVES.get() + 1;
        WAVES.set(n);
        WAVE.with_borrow_mut(|w| -> Result<(), String> {
            *w = Some((n, mj::Batch::new().map_err(|e| format!("{e:?}"))?));
            Ok(())
        })?;
        crate::code_rule_dsl::logic::lanes::wave::defer(Box::new(move || {
            let Some((n, mut batch)) = WAVE.with_borrow_mut(Option::take) else { return Ok(()) };
            let done = on_lane("stratum joins (a wave)", 1, || batch.finish().map_err(|e| format!("{e:?}")));
            // A failed wave fails its strata, which then run on the CPU: the flush itself goes on.
            FLUSHED.with_borrow_mut(|f| f.insert(n, done.map(Some)));
            Ok(())
        }));
    }
    let encoded = WAVE.with_borrow_mut(|w| {
        let (n, batch) = w.as_mut().expect("the open wave");
        encode_runs(rules, batch, &mut rep, names).map(|runs| (*n, runs))
    })?;
    let (wave, runs) = encoded;
    if runs.len() < rules.len() {
        // Encoded work it will not wait for: the wave runs it and nothing reads it, but it writes the runs' buffers,
        // which the wave keeps until it is done (back in the pool, another job's would be written).
        WAVE.with_borrow_mut(|w| w.as_mut().expect("the open wave").1.hold(Box::new(runs)));
        return Ok(None);
    }
    rep.encode_cpu = thread_cpu() - c0 - rep.upload_cpu;
    let t = TICKETS.get() + 1;
    TICKETS.set(t);
    SUSPENDED.with_borrow_mut(|s| s.insert(t, (runs, wave, rep)));
    Ok(Some(t))
}

/// A stratum that waited for its wave (`submit`'s `ticket`), finished: as `run`, from the flushed submission.
pub(in crate::code_rule_dsl) fn resume(ticket: u64, rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "device joins (after the wave)".into());
    let Some((runs, wave, mut rep)) = SUSPENDED.with_borrow_mut(|s| s.remove(&ticket)) else { return Err("a stratum's device joins were not submitted".into()) };
    let done = FLUSHED.with_borrow_mut(|f| match f.get_mut(&wave) {
        None => Err("a stratum's wave was not flushed".to_owned()),
        Some(Err(e)) => Err(e.clone()),
        // The wave's device seconds, counted once (by its first stratum).
        Some(Ok(d)) => Ok(d.take()),
    })?;
    if let Some(d) = done {
        rep.submissions += d.submissions;
        rep.dispatches += d.dispatches;
        rep.gpu_seconds += d.gpu_seconds;
        rep.sized += d.sized;
    }
    rep.waved = 1;
    let c0 = thread_cpu();
    let out = complete(runs, rules, store, names, fresh, &mut rep)?;
    rep.encode_cpu += (thread_cpu() - c0 - rep.decode_cpu).max(0.0);
    let rows: u64 = rep.rows;
    crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "head rows read back", rows);
    LAST.with_borrow_mut(|l| *l = Some(rep));
    Ok(out)
}

/// The first rules of `rules` that fit one submission's space, run: their head rows as `run`'s.
fn group(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh], rep: &mut Report) -> Result<Vec<u64>, String> {
    let mut batch = mj::Batch::new().map_err(|e| format!("{e:?}"))?;
    let runs = encode_runs(rules, &mut batch, rep, names)?;
    finish_timed(&mut batch, rep)?;
    complete(runs, &rules[..], store, names, fresh, rep)
}

/// The first rules of `rules` that fit one submission's space, each encoded into `batch`.
fn encode_runs(rules: &[(&Plan, &JoinRule, usize)], batch: &mut mj::Batch, rep: &mut Report, names: &Interner) -> Result<Vec<Run<'static>>, String> {
    // A lease past the Metal budget is refused here (decision 0126): the stratum then runs on the CPU, named.
    rerail_accel::gpu::shared::within_budget(|| encode_runs_bounded(rules, batch, rep, names))
}

fn encode_runs_bounded(rules: &[(&Plan, &JoinRule, usize)], batch: &mut mj::Batch, rep: &mut Report, names: &Interner) -> Result<Vec<Run<'static>>, String> {
    let _h = crate::code_rule_dsl::logic::lanes::prof::host("device join (decision 0095)", "encode: uploads, sorts, stages");
    let mut runs: Vec<Run<'static>> = Vec::with_capacity(rules.len());
    let mut bytes = 0;
    for (plan, _, _) in rules {
        if !runs.is_empty() && (bytes > batch_max() || mj::device_bytes_left() < headroom()) {
            break;
        }
        let mut r = match Run::new(plan) {
            Ok(r) => r,
            Err(e) => {
                batch.hold(Box::new(runs));
                return Err(e);
            }
        };
        if let Err(e) = r.encode(0, batch, rep, &mut Own::default(), names) {
            // The runs encoded so far (and this one's stages before the refusal) are in the batch: their buffers stay
            // with it until it is done.
            batch.hold(Box::new((runs, r)));
            return Err(e);
        }
        bytes += r.bytes();
        runs.push(r);
    }
    Ok(runs)
}

/// Commits `batch` and waits for it, the wait and the device's seconds into `rep`.
fn finish_timed(batch: &mut mj::Batch, rep: &mut Report) -> Result<(), String> {
    let (t0, w0) = (Instant::now(), thread_cpu());
    let done = batch.finish().map_err(|e| format!("{e:?}"))?;
    rep.wait_wall += t0.elapsed().as_secs_f64();
    rep.wait_cpu += thread_cpu() - w0;
    rep.submissions += done.submissions;
    rep.dispatches += done.dispatches;
    rep.gpu_seconds += done.gpu_seconds;
    rep.sized += done.sized;
    Ok(())
}

/// Runs whose submission completed, finished: an overflowing stage run again from its exact count (in a batch of
/// their own), then the rows into the store (`fresh`) and each step's counts. `rules` are the runs' rules, in order.
fn complete(mut runs: Vec<Run<'static>>, rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh], rep: &mut Report) -> Result<Vec<u64>, String> {
    let mut pending: Vec<usize> = (0..runs.len()).collect();
    let mut batch: Option<mj::Batch> = None;
    loop {
        let mut again = Vec::new();
        // An arena that filled: the bytes its jobs wanted (every run in its batch shares it), for the next batch's.
        let mut arena = 0;
        for &k in &pending {
            let r = &mut runs[k];
            let over = r.overflow();
            let wanted = r.arena.as_ref().map_or(0, |a| a.wanted());
            let full = r.arena_full() && wanted <= arena_max();
            if over.is_none() && !full {
                continue;
            }
            let mut from = usize::MAX;
            if let Some((s, n)) = over {
                let o = r.io[s].1;
                r.caps[o] = n;
                // The stages after it are sized again from the exact count.
                for later in s + 1..r.plan.stages.len() {
                    let (i, o2) = r.io[later];
                    if i != o2 {
                        r.caps[o2] = 0;
                    }
                }
                from = s;
            }
            if full {
                arena = arena.max(wanted);
            }
            // Its texts all in the next batch's arena: from its first text function, when that is before.
            if let Some(f) = r.first_textfn() {
                from = from.min(f);
            }
            again.push((k, from));
        }
        if again.is_empty() {
            if let Some(why) = runs.iter().find_map(Run::failed) {
                return Err(why);
            }
            break;
        }
        rep.reruns += again.len() as u32;
        pending = again.iter().map(|&(k, _)| k).collect();
        if arena > 0 {
            rep.arena_reruns += 1;
        }
        if arena > 0 || batch.is_some() && again.iter().any(|&(k, _)| runs[k].first_textfn().is_some()) {
            // Texts made again go into a new batch's arena (as big as the last one's jobs asked, when it filled).
            batch = None;
        }
        let b = match &mut batch {
            Some(b) => b,
            None => batch.insert(mj::Batch::with_arena(arena.max(mj::ARENA_START)).map_err(|e| format!("{e:?}"))?),
        };
        for (k, s) in again {
            runs[k].encode(s, b, rep, &mut Own::default(), names)?;
        }
        finish_timed(b, rep)?;
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
        // The arena texts in the head, stored as symbols, all at once.
        let arena: Vec<Option<HashMap<i64, i64>>> = plan.arena.iter().enumerate().map(|(c, &a)| -> Result<Option<HashMap<i64, i64>>, String> {
            if !a {
                return Ok(None);
            }
            let ar = r.arena.as_ref().ok_or("an arena text with no arena")?;
            let mut vals: Vec<i64> = cols[c].to_vec();
            vals.sort_unstable();
            vals.dedup();
            let strs = vals.iter().map(|&v| {
                let b = ar.text(v).ok_or("an arena text past its arena")?;
                std::str::from_utf8(b).map_err(|_| "an arena text that is not UTF-8".to_owned())
            }).collect::<Result<Vec<&str>, String>>()?;
            let ids = names.atoms.intern_all(&strs).map_err(|e| format!("the arena texts' symbols: {e}"))?;
            rep.made += strs.len() as u64;
            Ok(Some(vals.into_iter().zip(ids).map(|(v, id)| (v, i64::from(id))).collect()))
        }).collect::<Result<_, _>>()?;
        for x in 0..distinct {
            for (c, h) in plan.head.iter().enumerate() {
                row[c] = h.unwrap_or_else(|| cols[c][x]);
                if let Some(m) = &arena[c] {
                    row[c] = m[&row[c]];
                }
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
pub(in crate::code_rule_dsl) fn reads_own(plan: &Plan) -> bool {
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
        if let Err(e) = r.encode(0, &mut batch, rep, own, names) {
            // The batch's work writes the runs' buffers: done before they go back to the pool.
            let _ = batch.finish();
            return Err(e);
        }
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
        rep.sized += done.sized;
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
            if let Err(e) = runs[k].as_mut().expect("a run").encode(s, &mut batch, rep, own, names) {
                let _ = batch.finish();
                return Err(e);
            }
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
pub(in crate::code_rule_dsl) fn fixpoint(seeds: &[(&Plan, &JoinRule, usize)], deltas: &[(&Plan, &JoinRule, usize)], heads: &[usize], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    on_lane("stratum rounds", seeds.len() + deltas.len(), || fixpoint_here(seeds, deltas, heads, store, names, fresh))
}

fn fixpoint_here(seeds: &[(&Plan, &JoinRule, usize)], deltas: &[(&Plan, &JoinRule, usize)], heads: &[usize], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "device fixpoint".into());
    let c0 = thread_cpu();
    let mut rep = Report { rules: (seeds.len() + deltas.len()) as u32, stages: seeds.iter().chain(deltas).map(|r| r.0.stages.len() as u32).sum(), ..Report::default() };
    if seeds.iter().chain(deltas).any(|r| r.0.stages.iter().any(|s| makes_text(s.desc.mode))) {
        return Err("a made text in a recursive component (its rounds would join texts not yet symbols)".into());
    }
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
    // A lattice head folds each round by key on the device (decision 0124): its fold, and its key table.
    let mut lattices: HashMap<usize, (mj::LatCode, mj::LatKeys)> = HashMap::new();
    for &h in heads {
        if let Some(lat) = store.rels[h].device_lattice() {
            let (code, top, after) = lat?;
            lattices.insert(h, (mj::LatCode { code, top, after }, mj::LatKeys::default()));
        }
    }
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
        // A lattice head's round: its new keys' rows (with their change counts), its changed rows, their counts.
        let mut folded: Vec<(usize, mj::Table, mj::Table, mj::Words)> = Vec::new();
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
            if let Some((lat, keys)) = lattices.get_mut(&h) {
                let new_keys = mj::Table::new(k + 1, cap).map_err(|e| format!("{e:?}"))?;
                let changed = mj::Table::new(k, cap).map_err(|e| format!("{e:?}"))?;
                let counters = mj::Words::zeroed(2).map_err(|e| format!("{e:?}"))?;
                let total = own.total.get(&h).cloned();
                batch.lattice_fold(&union, cap, total.as_deref(), k - 1, *lat, keys, &counters, &new_keys, &changed).map_err(|e| format!("{e:?}"))?;
                folded.push((h, new_keys, changed, counters));
                hold.push(union);
                continue;
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
        rep.sized += done.sized;
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
        // A lattice head: its held rows were updated in place; its new keys' rows onto it, its changed rows its delta.
        for (h, new_keys, changed, counters) in folded {
            let (n, added) = (counters.get(0) as usize, counters.get(1) as usize);
            own.sorts.retain(|(r, _, _), _| *r != h);
            if n == 0 {
                own.delta.remove(&h);
                continue;
            }
            any = true;
            let k = store.rels[h].k;
            if added > 0 {
                let ident: Vec<mj::Col> = (0..=k).map(mj::Col::From).collect();
                let had = own.total.get(&h).map_or(0, |t| t.rows);
                let total = mj::DevRel::empty(k + 1, had + added).map_err(|e| format!("{e:?}: a relation of {} rows", had + added))?;
                if let Some(old) = own.total.get(&h) {
                    batch.copy_rows(mj::Rows::Rel(old), had, &ident, mj::Rows::Rel(&total), 0).map_err(|e| format!("{e:?}"))?;
                }
                batch.copy_rows(mj::Rows::Table(&new_keys), added, &ident, mj::Rows::Rel(&total), had).map_err(|e| format!("{e:?}"))?;
                old.extend(own.total.insert(h, Rc::new(total)));
            }
            let ident: Vec<mj::Col> = (0..k).map(mj::Col::From).collect();
            let delta = mj::DevRel::empty(k, n).map_err(|e| format!("{e:?}"))?;
            batch.copy_rows(mj::Rows::Table(&changed), n, &ident, mj::Rows::Rel(&delta), 0).map_err(|e| format!("{e:?}"))?;
            own.delta.insert(h, Rc::new(delta));
            rows_held.push(new_keys);
            rows_held.push(changed);
        }
        let done = batch.finish().map_err(|e| format!("{e:?}"))?;
        drop((old, rows_held));
        rep.submissions += done.submissions;
        rep.dispatches += done.dispatches;
        rep.gpu_seconds += done.gpu_seconds;
        rep.sized += done.sized;
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
            _ if s.span == 0 => Vec::new(),
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
mod parity;

#[cfg(test)]
pub mod tests {
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
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::content_store::ContentStore::open(&store).unwrap());
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
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::content_store::ContentStore::open(&store).unwrap());
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
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::content_store::ContentStore::open(&store).unwrap());
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
            // A cat's text compared: with a constant, with another relation's symbol, and not equal.
            format!("pos {n} v0 v1\ncat v2 v0 {}\neq v2 {}\nhead v0 v1", sym("ts"), sym("foo.ts")),
            format!("pos {m} v0\npos {n} v1 v2\ncat v3 v0 {}\neq v3 v1\nhead v1 v2", sym("ts")),
            format!("pos {n} v0 v1\nlt v1 i:100\ncat v2 v0 {}\nne v2 {}\nlt v1 i:90\nhead v0 v1", sym("x9"), sym("foo.x9")),
            // A concatenation compared (`str_concat`: no "." between, none dropped).
            format!("pos {n} v0 v1\nfunc str_concat v2 v0 {}\neq v2 {}\nhead v0 v1", sym("ts"), sym("foots")),
            format!("pos {m} v0\npos {n} v1 v2\nfunc str_concat v3 v0 {}\neq v3 v1\nhead v1 v2", sym("x9")),
            format!("pos {n} v0 v1\nfunc str_concat v2 {} v0\nne v2 {}\nhead v0 v1", sym("a"), sym("afoo")),
            // Texts made into the head, new symbols: a cat bound, a cat in the head (an empty part among them), a
            // concatenation.
            format!("pos {n} v0 v1\nlt v1 i:700\ncat v2 v0 {}\nhead v2 v1", sym("made")),
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:25\nhead cat:v0:v1 v2"),
            format!("pos {n} v0 v1\nlt v1 i:300\nfunc str_concat v2 v0 {}\nhead v2 v1", sym("!")),
            // Bound patterns, pairwise: a relation's names as prefixes, suffixes, equals of another's (an empty one
            // among them), and a cat's text as the pattern.
            format!("pos {m} v0\npos {n} v1 v2\ntest prefix v1 v0 1\nhead v1 v2"),
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:400\ntest suffix v1 v0 1\nhead v0 v2"),
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:300\ntest suffix v1 v0 0\nhead v0 v2"),
            format!("pos {m} v0\npos {n} v1 v2\ntest equals v1 v0 1\nhead v1 v2"),
            format!("pos {m} v0\npos {n} v1 v2\ncat v3 {} v0\ntest suffix v1 v3 1\nhead v1 v2", sym("")),
            format!("pos {m} v0\npos {n} v1 v2\nfunc str_concat v3 v0 {}\ntest prefix v1 v3 1\nhead v0 v2", sym("_")),
            // A part a let filled with a constant (`q = @dot("builtins", x)`).
            format!("pos {n} v0 v1\nlt v1 i:200\neq v2 {}\ncat v3 v2 v0\nhead v3 v1", sym("builtins")),
            // A cat's text ordered against a name, and its characters.
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:200\ncat v3 v0 {}\nfunc str_cmp v4 v3 v1\nlt v4 i:0\nhead v1 v2", sym("x9")),
            format!("pos {m} v0\nfunc str_concat v1 v0 {}\nfunc rr_strlen v2 v1\nhead v0 v2", sym("αβ")),
            // Characters of names (multibyte ones among them, names past a word): strlen bound and compared.
            format!("pos {n} v0 v1\nlt v1 i:300\nfunc rr_strlen v2 v0\nhead v0 v2"),
            format!("pos {m} v0\nfunc rr_strlen v1 v0\nlt i:9 v1\nhead v0 v1"),
            // A cat's text both compared and in the head.
            format!("pos {m} v0\npos {n} v1 v2\nlt v2 i:500\ncat v3 v0 {}\nne v3 v1\ntest prefix v1 v0 1\nhead v3 v2", sym("ts")),
        ];
        let mut got = Vec::new();
        // The device first: the texts it makes are new symbols.
        for device in [true, false] {
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
            if device {
                assert!(report.as_ref().is_some_and(|r| r.made > 1000), "texts made on the device: {report:?}");
            }
            let counts = super::super::step_counts(&db, "dev_text").unwrap();
            super::super::close(&db, "dev_text").unwrap();
            let mut rows: Vec<(i64, i64)> = db.prepare("SELECT c0, c1 FROM temp.h0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            rows.sort_unstable();
            got.push((rows, counts));
        }
        assert!(got[0].0.len() > 500, "{} rows", got[0].0.len());
        let longest: i64 = db.query_row("SELECT max(length(CAST(c0 AS BLOB))) FROM temp.n", [], |r| r.get(0)).unwrap();
        assert!(longest > 64, "a name past two chunks: {longest} bytes");
        assert_eq!(got[1].0, got[0].0, "the device's text tests are the CPU's");
        assert_eq!(got[1].1, got[0].1, "the device's counts per step are the CPU's");
    }

    /// Strata placed on the device, when their driver can wait, go out in one wave: each encodes its joins into the
    /// level's one command buffer and returns `3`; one flush commits it; each then finishes, with the rows it makes
    /// on its own.
    #[test]
    fn strata_on_the_device_wait_for_their_wave() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        use crate::code_rule_dsl::logic::lanes::wave;
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::content_store::ContentStore::open(&store).unwrap());
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        db.execute_batch("CREATE TEMP TABLE n (c0, c1);").unwrap();
        for k in 0..400 {
            db.execute("INSERT INTO n VALUES (?1, ?2)", rusqlite::params![format!("s{}", k % 37), k]).unwrap();
        }
        let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
        let strata = [
            ("wave_a", "h0", format!("pos {n} v0 v1\nlt v1 i:100\nhead v0 v1")),
            ("wave_b", "h1", format!("pos {n} v0 v1\npos {n} v0 v2\nlt v1 v2\nlt v2 i:60\nhead v1 v2")),
            ("wave_c", "h2", format!("pos {n} v0 v1\ncat v2 v0 x:{}\nhead v2 v1", hex("w"))),
        ];
        let mut got = Vec::new();
        for waves in [false, true] {
            for (name, h, rule) in &strata {
                db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{h}; CREATE TEMP TABLE {h} (c0, c1);")).unwrap();
                super::super::open(&db, &format!("name {name}\ndevice\nrel {h} si empty\nseed 0 {}", hex(rule))).unwrap();
            }
            if waves {
                let (w0, _) = wave::flushed();
                {
                    let _w = wave::suspendable();
                    for (name, _, _) in &strata {
                        let status = super::super::run(&db, name, 0).unwrap();
                        assert!(status.ends_with(" 3"), "{name} waits for its wave: {status}");
                    }
                }
                assert_eq!(wave::flush(strata.len()).unwrap(), 1, "one commit for the level's strata");
                assert_eq!(wave::flushed().0, w0 + 1);
                for (name, _, _) in &strata {
                    let status = super::super::run(&db, name, 0).unwrap();
                    assert!(!status.ends_with(" 3"), "{name} finished: {status}");
                    let report = super::take_report().expect("a device report");
                    assert!(report.refused.is_none() && report.waved == 1, "{report:?}");
                }
            } else {
                for (name, _, _) in &strata {
                    super::super::run(&db, name, 0).unwrap();
                    assert!(super::take_report().is_some_and(|r| r.refused.is_none() && r.waved == 0));
                }
            }
            let mut rows = Vec::new();
            for (name, h, _) in &strata {
                super::super::close(&db, name).unwrap();
                let mut r: Vec<(i64, i64)> = db.prepare(&format!("SELECT c0, c1 FROM temp.{h}")).unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
                r.sort_unstable();
                assert!(!r.is_empty(), "{name} made rows");
                rows.push(r);
            }
            got.push(rows);
        }
        assert_eq!(got[1], got[0], "the strata's rows after the wave are their own runs'");
    }

    /// Text functions on the device: their texts made into the batch's arena (an arena too small run again in one
    /// as big as its texts asked), read by tests, `str_cmp`, `strlen` and other functions, stored as symbols in the
    /// head: the CPU's rows. `lower` of a name past ASCII is the host's: the stratum runs on the CPU.
    #[test]
    fn text_functions_on_the_device_are_the_cpus() {
        if !crate::code_rule_dsl::transducer::packed::metal_join::available() {
            return;
        }
        let store = crate::code_rule_dsl::fixtures::temp();
        drop(crate::content_store::ContentStore::open(&store).unwrap());
        let db = Connection::open(&store).unwrap();
        crate::code_rule_dsl::logic::native::register(&db).unwrap();
        crate::code_rule_dsl::logic::native::register_symbols(&db).unwrap();
        let mut x = 7u64;
        let mut next = |m: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) % m
        };
        let parts = ["foo", "Bar", "SRC", "src/lib", "a.b", "_", ".ts", "αβ", "x9", "TODO", "a_b", "/", "é"];
        db.execute_batch("CREATE TEMP TABLE n (c0, c1); CREATE TEMP TABLE m (c0); CREATE TEMP TABLE a (c0, c1);").unwrap();
        for k in 0..2000 {
            let len = 1 + next(if k % 50 == 0 { 30 } else { 6 });
            let name: String = (0..len).map(|_| parts[next(parts.len() as u64) as usize]).collect();
            db.execute("INSERT INTO n VALUES (?1, ?2)", rusqlite::params![if k == 7 { String::new() } else { name }, k]).unwrap();
        }
        for k in 0..80 {
            let len = 1 + next(3);
            let name: String = (0..len).map(|_| parts[next(5) as usize]).collect();
            db.execute("INSERT INTO m VALUES (?1)", rusqlite::params![name]).unwrap();
            db.execute("INSERT INTO a VALUES (?1, ?2)", rusqlite::params![name.to_ascii_uppercase(), k]).unwrap();
        }
        let n = format!("t:x:{}:{}:si", hex("temp.n"), hex("c0, c1"));
        let m = format!("t:x:{}:{}:s", hex("temp.m"), hex("c0"));
        let a = format!("t:x:{}:{}:si", hex("temp.a"), hex("c0, c1"));
        let sym = |t: &str| format!("x:{}", hex(t));
        let rules = [
            format!("pos {n} v0 v1\nlt v1 i:800\nfunc sql_lower v2 v0\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc sql_upper v2 v0\ntest prefix v2 {} 1\nhead v0 v1", sym("SRC")),
            format!("pos {n} v0 v1\nfunc head v2 v0\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc dir v2 v0\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc rr_substr v2 v0 i:1 i:4\nhead v2 v1"),
            format!("pos {n} v0 v1\nlt v1 i:40\nfunc slice v2 v0 i:-3 v1\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc sql_substr v2 v0 i:-2 i:5\nhead v2 v1"),
            format!("pos {n} v0 v1\nlt v1 i:500\nfunc rr_sub v3 i:250 v1\nfunc rr_to_string v2 v3\nhead v2 v1"),
            format!("pos {n} v0 v1\nfunc sql_replace v2 v0 {} {}\nhead v2 v1", sym("_"), sym("--")),
            // Chained: a range of a name, made upper, its characters counted.
            format!("pos {n} v0 v1\nfunc rr_substr v2 v0 i:0 i:6\nfunc sql_upper v3 v2\nfunc rr_strlen v4 v3\nlt v4 i:5\nhead v3 v1"),
            // Compared with another relation's symbols, and ordered against a constant.
            format!("pos {m} v0\npos {n} v1 v2\nfunc head v3 v1\neq v3 v0\nhead v1 v2"),
            format!("pos {n} v0 v1\nfunc dir v2 v0\nfunc str_cmp v3 v2 {}\nlt i:0 v3\nhead v0 v1", sym("src")),
            // Regex replacements: leftmost first, anchored, multibyte classes.
            format!("pos {n} v0 v1\nfunc replace_all v2 v0 {} {}\nhead v2 v1", sym("_+|\\.ts$"), sym("-")),
            format!("pos {n} v0 v1\nfunc replace_all v2 v0 {} {}\nhead v2 v1", sym("^[A-Za-z]+|[0-9]"), sym("#")),
            // Matches past a chunk of 32 bytes (long names, long runs), their ends and starts found across chunks.
            format!("pos {n} v0 v1\nfunc replace_all v2 v0 {} {}\nhead v2 v1", sym("[A-Za-z_./αβ]{3,}"), sym("~")),
            format!("pos {n} v0 v1\nlt v1 i:900\nfunc rx_replace v2 v0 {} {}\nfunc rr_strlen v3 v2\nhead v2 v3", sym("[αβé]+"), sym("<u>")),
            // Unicode's lower of ASCII names.
            format!("pos {a} v0 v1\nfunc lower v2 v0\nhead v2 v1"),
        ];
        let lowered = [format!("pos {n} v0 v1\nfunc lower v2 v0\nhead v2 v1")];
        for (case, rules, on_device) in [("arena", &rules[..], true), ("past ASCII", &lowered[..], false)] {
            let mut got = Vec::new();
            for device in [true, false] {
                db.execute_batch("DROP TABLE IF EXISTS temp.h0; CREATE TEMP TABLE h0 (c0, c1);").unwrap();
                let mut spec = vec!["name dev_fn".to_owned()];
                if device {
                    spec.push("device".into());
                }
                spec.push("rel h0 si empty".into());
                spec.extend(rules.iter().map(|r| format!("seed 0 {}", hex(r))));
                super::super::open(&db, &spec.join("\n")).unwrap();
                super::super::run(&db, "dev_fn", 0).unwrap();
                let report = super::take_report();
                if device {
                    let r = report.as_ref().expect("a device report");
                    assert_eq!(r.refused.is_none(), on_device, "{case}: {r:?}");
                    if on_device {
                        assert!(r.made > 2000 && r.arena_reruns >= 1, "{case}: texts made in an arena that filled once: {r:?}");
                        assert!(r.per_symbol >= 12, "{case}: text functions made once per distinct name, chunked: {r:?}");
                    }
                }
                let counts = super::super::step_counts(&db, "dev_fn").unwrap();
                super::super::close(&db, "dev_fn").unwrap();
                let mut rows: Vec<(String, i64)> = db
                    .prepare("SELECT sym_text(c0), c1 FROM temp.h0")
                    .unwrap()
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                rows.sort_unstable();
                got.push((rows, counts));
            }
            assert!(got[0].0.len() > 100, "{case}: {} rows", got[0].0.len());
            assert_eq!(got[1].0, got[0].0, "{case}: the device's text functions are the CPU's");
            assert_eq!(got[1].1, got[0].1, "{case}: the device's counts per step are the CPU's");
        }
    }
}
