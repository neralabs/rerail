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
//! **One submission per stratum.** Every seed join of the stratum is encoded into one `metal_join::Batch`; no device
//! cache holds a relation across strata (decision 0450 step 3: a view's indexes are in its own block). A stage
//! whose rows overflow the space its table was given is run again from that stage with the exact size, in one more
//! submission for every rule that overflowed.
//!
//! **The same result.** The head rows go into the store exactly as the CPU's do (a row already there, or found twice,
//! is dropped by the store's own set), and each step's observed counts (the bindings that entered it and that it
//! passed on) are the CPU's, so the replanner sees the same fanouts.

use super::{InRel, Interner, JoinRule, Key, Src, Step, Store, Term, Fresh, is_dead, rowcode, thread_cpu};
use crate::code_rule_dsl::transducer::packed::metal_join as mj;
use crate::code_rule_dsl::logic::device::{ColumnStats, StatColumn};
use crate::code_rule_dsl::logic::device::caps;
use crate::code_rule_dsl::logic::profile;
use crate::code_rule_dsl::facts::rows::{BlockVec, Part};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::time::Instant;
pub use crate::code_rule_dsl::logic::device::{Report};

/// The most rows a table is given before its stage has run once (a guess; an overflow runs it again exactly).
const GUESS_MAX: usize = 1 << 21;
impl Plan {
    /// The bytes its tables are expected to take and the rows its stages are expected to make (its planned work),
    /// from the planner's estimates (`est`, sized as `table_rows` sizes them): what its stratum reserves before it
    /// encodes (`admit`). A stage without an estimate counts none.
    fn expected(&self) -> (usize, f64) {
        let row = 8 * self.nslots.max(1);
        let most = table_max() / row;
        self.est.iter().flatten().fold((0, 0.0), |(bytes, work), &e| (bytes + row * (((e * EST_MARGIN).ceil() as usize).max(16)).min(most), work + e))
    }
}

/// The reservation for `rules`' tables (`admit`), waited for while waiting is worth what the CPU would take over
/// them (`placement::worth_waiting`); its wait counted in `rep`.
fn admitted(rules: &[(&Plan, &JoinRule, usize)], rep: &mut Report) -> Result<rerail_accel::gpu::shared::Admitted, String> {
    let (bytes, work) = rules.iter().map(|r| r.0.expected()).fold((0, 0.0), |(b, w), (pb, pw)| (b + pb, w + pw));
    let patience = crate::code_rule_dsl::logic::plan::placement::admission_patience(work);
    let got = rerail_accel::gpu::shared::admit(bytes, patience)?;
    if got.waited > 0.0 {
        rep.admit_waits += 1;
        rep.admit_seconds += got.waited;
        crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "strata that waited for the device's memory", 1);
    }
    Ok(got)
}

/// A join stage's table, in rows, before it runs: the growth the stage was seen to have (`seen`, rows out a row in);
/// else the planner's estimate of its rows (`est`, `JoinRule::est`) with a margin; else twice its input (`cap_in`).
/// Q203: doubling each stage's input compounded down a plan, and a cold run's tables were a hundred times their rows,
/// refused past the device's budget. A `scan` (no keys, one binding in) holds its relation's rows (`nr`) at most. An
/// overflow runs the stage again exactly.
fn table_rows(cap_in: usize, nr: usize, scan: bool, seen: Option<f64>, est: Option<f64>) -> usize {
    let seen = seen.map(|r| ((cap_in as f64 * r * 1.25).ceil() as usize).max(16));
    let est = est.filter(|_| !rerail_accel::gpu::shared::q180_off("est")).map(|e| ((e * EST_MARGIN).ceil() as usize).max(16));
    if scan {
        return seen.or(est).map_or(nr, |e| e.min(nr)).max(1);
    }
    seen.or(est).unwrap_or(cap_in.saturating_mul(2)).min(cap_in.saturating_mul(nr.max(1))).clamp(1, GUESS_MAX.max(cap_in))
}

/// A table sized by the planner's estimate holds this many times it: the estimates miss by about that (rr_token_req's
/// first step: 2,848 expected, 7,453 made), and an overflow runs the stage again exactly.
const EST_MARGIN: f64 = 2.0;
/// The device's buffers share the Metal budget (`metal_buffer_limit`, 512 MB by default; decision 0126) with the
/// buffer pool and the other lanes. The device joins keep within about a quarter of it at the default (their sizes
/// below are those at 512 MB, scaled with the budget, `metal_share`):
/// the most bytes one binding table may take: past it the rule is refused at run time and runs on the CPU, named;
fn table_max() -> usize { rerail_accel::budget::metal_share(48 << 20) }
/// the most bytes one submission's tables may take: past it the rules after go into the next submission;
fn batch_max() -> usize { rerail_accel::budget::metal_share(64 << 20) }
/// The largest text arena a batch runs again with; past it, the stratum's texts are made on the host.
fn arena_max() -> usize { rerail_accel::budget::metal_share(64 << 20) }
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
    /// A text test of a symbol by its table, its verdicts in the store's one rolling cache (decision 0478 step 2d).
    Verdict { table: Arc<crate::code_rule_dsl::text::table::Table>, test: u32 },
    /// A stage that reads names (`str_cmp`, a cat compared): its symbols' bytes read in place through the batch's
    /// atoms directory (decision 0478 step 2e).
    Names,
    /// A made text per row (`mj::ROWTEXT`: a cat, a regex's replacement, a text function, a text test of one), a SIMD
    /// group a row, its symbols' bytes read in place (decision 0478 step 2e), with its automaton; `kept`, the id its
    /// text is kept by in the one rolling cache (`mj::ROW_KEPT`), with the source symbol (0 when it keeps none).
    RowText { auto: RowAuto, kept: u32 },
    /// The rule's steps past the one that binds the head (`setfix::tail`): an atom over `rel` (by its key columns
    /// `cols`) and the atoms after it (`rest`: each relation, its key columns and its probe), run as one witness per
    /// binding (`mj::WITNESS`, decision 0277), as the CPU's walk runs them (decision 0229).
    Witness { rel: Arc<InRel>, cols: Vec<usize>, rest: Vec<(Arc<InRel>, Vec<usize>, mj::Stage)> },
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

/// Whether a stage of `mode` reads symbols' texts (through the batch's atoms directory, decision 0478 step 2e): a made
/// text, a text op, `str_cmp`. Each has a counter word for what it finds the host's.
fn reads_names(mode: u32) -> bool {
    matches!(mode, mj::ROWTEXT | mj::TEXTOP | mj::STRCMP)
}

/// The batch's atoms directory, made (or made again, the store's atoms grown) for a stage that reads symbols' texts.
fn dev_atoms(batch: &mut mj::Batch, names: &Interner) -> Result<(), String> {
    let atoms = &names.atoms;
    let runs = atoms.runs();
    let end = runs.iter().map(|(r, _)| r.end).max().unwrap_or(0);
    batch.atoms(end, || mj::DevAtoms::new(runs)).map_err(|e| format!("{e:?}: the atoms directory"))?;
    batch.kept(|| mj::DevKept::new(atoms.kept())).map_err(|e| format!("{e:?}: the rolling cache"))
}

/// A `ROWTEXT` stage of op `op` (`mj::ROW_*`, with a test's verdict wanted at bit 8) over operands `x` and `y` (a
/// symbol, a held cat's text, an arena text).
fn row_stage(first: usize, op: u32, x: Key, y: Key, auto: RowAuto, deferred: &HashMap<usize, (Key, Key, u32)>, origin: &[Option<(Arc<InRel>, usize)>], known: &HashMap<usize, i64>) -> Result<PStage, String> {
    let mut st = text_stage(mj::ROWTEXT, first, op, x, y, deferred, origin, known)?;
    if !matches!(st.kind, Kind::Names) {
        return Err("a made text's operands of no names".into());
    }
    st.kind = Kind::RowText { auto, kept: 0 };
    Ok(st)
}

/// The id a chain of text functions keeps its texts by (`keep_text`'s: a step `/{function}{constants:?}`, by the
/// rule's function names, so the CPU names a chain as the device does): process-wide, by the automata block's index.
pub(in crate::code_rule_dsl) fn text_fns_id(chain: &str) -> Option<u32> {
    crate::code_rule_dsl::text::automata::text_functions(chain)
}

/// What a text function stage `d` of `x` keeps its text by (decision 0478 step 2e): when `x` is an input column's
/// symbol, or a kept text of one (`kept`'s, by its slot), and every other argument a constant, the chain of functions
/// from that symbol (`step` the last, its constants in it) and its id. The stage then looks its text up by (id,
/// source symbol, key 7) in the one rolling cache, and keeps what it makes (`mj::ROW_KEPT`).
fn keep_text(d: &mut mj::Stage, x: Key, consts: Option<Vec<i64>>, step: &str, origin: &[Option<(Arc<InRel>, usize)>], kept: &HashMap<usize, (usize, String)>) -> Option<(u32, (usize, String))> {
    let consts = consts?;
    let Key::Slot(x) = x else { return None };
    let (src, chain) = match d.agg[1] & 15 {
        mj::OPND_SYM if origin.get(x).is_some_and(Option::is_some) => (x, String::new()),
        mj::OPND_ARENA => kept.get(&x).cloned()?,
        _ => return None,
    };
    let chain = format!("{chain}/{step}{consts:?}");
    let id = text_fns_id(&chain)?;
    d.agg[0] |= mj::ROW_KEPT;
    d.key_kind[7] = mj::SLOT;
    d.key_slot[7] = src as u32;
    Some((id, (src, chain)))
}

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

/// A text function the device makes (`mj::TEXTFN_*`), its text arguments then its numbers (`caps`' table).
fn text_function(name: &str) -> Option<(u32, usize, usize)> {
    use caps::TextFn::*;
    let caps::DeviceFn::Text { f, texts, nums } = caps::device_fn(name)? else { return None };
    let f = match f {
        Lower => mj::TEXTFN_LOWER,
        AsciiLower => mj::TEXTFN_ASCII_LOWER,
        AsciiUpper => mj::TEXTFN_ASCII_UPPER,
        Head => mj::TEXTFN_HEAD,
        Dir => mj::TEXTFN_DIR,
        Substr => mj::TEXTFN_SUBSTR,
        SqlSubstr => mj::TEXTFN_SQL_SUBSTR,
        Slice => mj::TEXTFN_SLICE,
        ToString => mj::TEXTFN_TO_STRING,
        ZeroPad => mj::TEXTFN_ZERO_PAD,
        Replace => mj::TEXTFN_REPLACE,
    };
    Some((f, texts, nums))
}

/// The texts of `keys` (slots an input column bound, or constants), for a stage that reads names: the columns and
/// the constants, into `d`'s keys `0..` as slots or constants.
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
    // The symbols' bytes are read where the atoms blocks hold them: nothing of the columns is uploaded.
    let _ = (cols, consts);
    Ok(Kind::Names)
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

/// What bounds a head column's values: a column of an input relation (`InRel::col_range`), or of a relation of the
/// component (its rules' bounds, to a fixed point).
#[derive(Clone)]
pub(in crate::code_rule_dsl) enum HeadFrom {
    In(Arc<InRel>, usize),
    Own(usize, usize),
}

/// A join rule's device plan.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Plan {
    stages: Vec<PStage>,
    head: Vec<Option<i64>>,
    head_slots: Vec<usize>,
    /// Per head column, what bounds its values: a column of an input relation or of the component's own, or none (a
    /// constant's column, or a value computed). The head's sort packs its key to these bounds (`head_bounds`).
    head_from: Vec<Option<HeadFrom>>,
    /// Per head column, whether its values are arena texts (a text function's), stored as symbols as the rows are.
    arena: Vec<bool>,
    nslots: usize,
    nsteps: usize,
    /// The rule's spec, hashed: its stages' observed fanouts are kept by it (`GROWTH`), for the next chunk's guesses.
    key: u64,
    /// Per stage, the rows the planner expects out of it (its last step's, `JoinRule::est`): its table's size before
    /// a growth has been seen.
    est: Vec<Option<f64>>,
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
    use caps::Arith::*;
    let caps::DeviceFn::Arith(op) = caps::device_fn(name)? else { return None };
    Some(match op {
        Add => mj::ARITH_ADD,
        Sub => mj::ARITH_SUB,
        Mul => mj::ARITH_MUL,
        Div => mj::ARITH_DIV,
        Mod => mj::ARITH_MOD,
        Max => mj::ARITH_MAX,
        Min => mj::ARITH_MIN,
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
        // A kind-set axis or containment is made whole by the batch's solve, and read so (decision 0277).
        // A `has` axis probed by one key is read off the tape (decision 0353 step 5, `resident`): nothing is made.
        Some(v) if keys > 0 && v.src.made_on_lookup() && !v.src.complete_rows() && !(keys == 1 && rel.ndead == 0 && v.src.has_walk().is_some()) => Err("an atom read by keys over a relation made on lookup".into()),
        _ => Ok(()),
    }
}

/// The witness form of `j`'s steps from `i` (decision 0277), where every slot the head reads is bound before `i`
/// (`setfix::tail`): an atom over an input relation that binds slots, then at most `mj::WITNESS_PROBES` atoms (or
/// negated atoms) over input relations keyed by slots bound before `i`, constants or the first atom's columns, none
/// with a fused program. Each binding needs one row of them, not all: the device's breadth-first stages would make
/// every one (PostHog: 10.6 M rows for 7,086 documents' keys at one step). `None` for any other shape.
fn witness_stage(j: &JoinRule, i: usize, bound: &[bool]) -> Option<PStage> {
    if i < super::tail(j) {
        return None;
    }
    let Step::Atom { src: Src::In(rel, _), neg: false, keys, binds, tests } = &j.steps[i] else { return None };
    let rest = &j.steps[i + 1..];
    if binds.is_empty() || j.fused[i].is_some() || rest.len() > mj::WITNESS_PROBES || keys.len() > mj::MAX_KEYS || tests.len() > mj::MAX_TESTS {
        return None;
    }
    readable(rel, keys.len()).ok()?;
    let key_of = |d: &mut mj::Stage, k: usize, c: usize, key: Key| {
        d.key_rcol[k] = c as u32;
        match key {
            Key::Slot(s) => { d.key_kind[k] = mj::SLOT; d.key_slot[k] = s as u32; }
            Key::Const(v) => { d.key_kind[k] = mj::CONST; d.key_const[k] = v; }
        }
    };
    let tests_of = |d: &mut mj::Stage, tests: &[(usize, usize)]| {
        for (k, &(a, b)) in tests.iter().enumerate() {
            d.test_a[k] = a as u32;
            d.test_b[k] = b as u32;
        }
        d.ntests = tests.len() as u32;
    };
    let mut d = mj::Stage { mode: mj::WITNESS, ..mj::Stage::default() };
    for (k, &(c, key)) in keys.iter().enumerate() {
        key_of(&mut d, k, c, key);
    }
    d.nkeys = keys.len() as u32;
    tests_of(&mut d, tests);
    let mut probes = Vec::new();
    for (m, st) in rest.iter().enumerate() {
        let Step::Atom { src: Src::In(r, _), neg, keys: bk, tests: bt, .. } = st else { return None };
        if j.fused[i + 1 + m].is_some() || bk.len() > mj::MAX_KEYS || bt.len() > mj::MAX_TESTS || readable(r, bk.len()).is_err() {
            return None;
        }
        let mut b = mj::Stage { mode: if *neg { mj::ANTI } else { mj::SEMI }, ..mj::Stage::default() };
        for (k, &(c, key)) in bk.iter().enumerate() {
            key_of(&mut b, k, c, key);
            if let Key::Slot(s) = key && !bound.get(s).copied().unwrap_or(false) {
                // A slot the first atom binds: its column of the candidate row.
                let &(ca, _) = binds.iter().find(|&&(_, x)| x == s)?;
                b.key_kind[k] = mj::ACOL;
                b.key_slot[k] = ca as u32;
            }
        }
        b.nkeys = bk.len() as u32;
        tests_of(&mut b, bt);
        probes.push((r.clone(), bk.iter().map(|&(c, _)| c).collect(), b));
    }
    let cols = keys.iter().map(|&(c, _)| c).collect();
    Some(PStage { first: i, span: j.steps.len() - i, kind: Kind::Witness { rel: rel.clone(), cols, rest: probes }, desc: d, npassed: 0 })
}

/// `j`'s device plan, or the step the device cannot run. `own_texts`: the component's own columns whose values are
/// arena texts (decision 0307: a `shortlex_min` relation's value, made by its rules' rounds into one arena), as
/// (relation, column).
pub(in crate::code_rule_dsl) fn compile(j: &JoinRule, spec: &str, own_texts: &[(usize, usize)]) -> Result<Plan, String> {
    let mut bound = vec![false; j.nslots];
    // Where each slot's value came from: a column of an input relation (a text test's terms are its symbols).
    let mut origin: Vec<Option<(Arc<InRel>, usize)>> = vec![None; j.nslots];
    // Where a slot's value came from when it is a column of the component's own relation (relation, column): with
    // `origin`, what bounds a head column's values (`Plan::head_from`).
    let mut own_origin: Vec<Option<(usize, usize)>> = vec![None; j.nslots];
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
    // `cat`s held back, by the slot they bind: each read only by an `eq` or `ne` after, which compares names.
    let mut deferred: HashMap<usize, (Key, Key, u32)> = HashMap::new();
    // The arena texts kept in the rolling cache: by slot, the source symbol's slot and the chain of functions.
    let mut kept_texts: HashMap<usize, (usize, String)> = HashMap::new();
    let mut stages = Vec::new();
    let mut i = 0;
    while i < j.steps.len() {
        let step = &j.steps[i];
        // The rest of the body as one witness per binding, where it takes that shape (decision 0277).
        if matches!(step, Step::Atom { .. }) && let Some(w) = witness_stage(j, i, &bound) {
            stages.push(w);
            i = j.steps.len();
            continue;
        }
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
                // An own column of arena texts (decision 0307) is read as the binding's text, never matched by value:
                // one text made in two rounds has two values.
                if let Some((rel, _)) = own && own_texts.iter().any(|&(r, _)| r == rel) {
                    let text = |c: usize| own_texts.contains(&(rel, c));
                    if keys.iter().any(|&(c, _)| text(c)) || tests.iter().any(|&(a, b)| text(a) || text(b)) || j.fused[i].is_some() || *neg {
                        return Err("a made text across rounds matched, tested or negated (decision 0307: read only as a text)".into());
                    }
                    for &(_, s) in atom_binds.iter().filter(|&&(c, _)| text(c)) {
                        ARENA.with_borrow_mut(|x| x.push(s));
                    }
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
                        if s >= own_origin.len() {
                            own_origin.resize(s + 1, None);
                        }
                        own_origin[s] = match src { Src::Delta(r) | Src::Full(r) | Src::FullMulti(r, _) => Some((*r, c)), Src::In(..) => None };
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
            // A min or max bounded by its probe (decision 0511): the device has no fold that reads the bound, so the
            // stratum stays on the CPU. Folding it as a plain min or max gave the group's extreme past the bound.
            Step::Scalar { bound: Some(_), .. } => return Err("an aggregate bounded by its probe (decision 0511)".into()),
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
                // word at a time, pairwise, as `strings::test` does: a constant set split into its members, a bound
                // pattern holding the set separator refused (`TEXT_BOUND`: no set).
                let bound = if matches!(p, Key::Slot(_)) { mj::TEXT_BOUND } else { 0 };
                let op = literal_test(op).expect("a literal test") | mj::TEXT_SPLIT | bound;
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
                // A text test with constant patterns, of a slot an input relation's column bound (a symbol): its
                // verdict read from the store's one rolling cache, else made by the test's automaton over the symbol's
                // bytes where the atoms blocks hold them, and kept there (decision 0478 step 2d).
                let Some(m) = matcher else { return Err(format!("the text test @{op} of a pattern that is not a constant has no device form")) };
                let Key::Slot(x) = *s else { return Err(format!("the text test @{op} of a constant has no device form")) };
                if origin.get(x).cloned().flatten().is_none() {
                    return Err(format!("the text test @{op} of a slot no input relation's column bound has no device form"));
                }
                let table = crate::code_rule_dsl::text::table::table(&m.plans).map_err(|e| format!("the text test @{op} has no device form: {e}"))?;
                let test = crate::code_rule_dsl::text::table::test_id(&m.plans).ok_or("the automata block's index is full")?;
                let mut d = mj::Stage { mode: mj::ROWTEXT, ..mj::Stage::default() };
                d.agg = [mj::ROW_VERDICT | u32::from(*want) << 8, 0];
                d.nkeys = 1;
                d.key_kind[0] = 1;
                d.key_slot[0] = x as u32;
                stages.push(PStage { first: i, span: 1, kind: Kind::Verdict { table, test }, desc: d, npassed: 0 });
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
                    if out >= own_origin.len() {
                        own_origin.resize(out + 1, None);
                    }
                    own_origin[out] = match (doc, src) { (false, Key::Slot(x)) => own_origin.get(x).copied().flatten(), _ => None };
                    if matches!(src, Key::Slot(x) if is_arena(x)) {
                        if doc {
                            return Err("a document of an arena text has no device form".into());
                        }
                        ARENA.with_borrow_mut(|a| a.push(out));
                    }
                    match (doc, src) {
                        (false, Key::Const(c)) => known.insert(out, c),
                        (false, Key::Slot(x)) => known.get(&x).copied().and_then(|c| known.insert(out, c)),
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
            Step::Func { name, args, out, bind: true, .. } if matches!(caps::device_fn(name), Some(caps::DeviceFn::Names { .. })) => {
                // A function of names: the order of two (compared a word of eight bytes at a time) or one's
                // characters, their texts those of the columns that bound the slots (and the constants').
                let Some(caps::DeviceFn::Names { f, .. }) = caps::device_fn(name) else { unreachable!("a function of names") };
                let f = match f { caps::Names::Cmp => mj::NAMES_CMP, caps::Names::Len => mj::NAMES_LEN };
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
            Step::Func { name: name @ ("replace_all" | "rx_replace"), args, out, bind: true, .. } if args.len() == 3 && matches!(args[0], Key::Slot(_)) => {
                // A regex's replacement in a symbol's text (read in place, decision 0478 step 2e) or a made text (an
                // arena text, a held cat's): per row, a SIMD group a row, 32 bytes at a time (`ROWTEXT`), into the
                // arena.
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
                if let Some((id, chain)) = keep_text(&mut st.desc, args[0], Some(vec![pattern, with]), name, &origin, &kept_texts) {
                    st.kind = Kind::RowText { auto: RowAuto::Replace { pattern, with }, kept: id };
                    kept_texts.insert(*out, chain);
                }
                stages.push(st);
                if *out >= bound.len() {
                    bound.resize(*out + 1, false);
                }
                bound[*out] = true;
                ARENA.with_borrow_mut(|a| a.push(*out));
                i += 1;
            }
            Step::Func { name: name @ ("replace_all" | "rx_replace"), .. } => {
                return Err(format!("the function {name} of a constant text, or of a regex or replacement not a constant, has no device form"));
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
                    Kind::Names
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
                let konst = |k: &Key| match *k {
                    Key::Const(c) => Some(c),
                    Key::Slot(x) => known.get(&x).copied(),
                };
                let mut kind = kind;
                if ntexts > 0
                    && let Some((id, chain)) = keep_text(&mut d, args[0], args[1..].iter().map(konst).collect(), name, &origin, &kept_texts) {
                        kind = Kind::RowText { auto: RowAuto::None, kept: id };
                        kept_texts.insert(*out, chain);
                    }
                if *out >= bound.len() {
                    bound.resize(*out + 1, false);
                }
                bound[*out] = true;
                ARENA.with_borrow_mut(|a| a.push(*out));
                stages.push(PStage { first: i, span: 1, kind, desc: d, npassed: 0 });
                i += 1;
            }
            Step::Func { name, kinds, args, out, bind, .. } => {
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
    let mut head_from = Vec::new();
    let mut arena = Vec::new();
    let mut nslots = j.nslots;
    for t in &j.out {
        match *t {
            Term::Slot(s) => {
                head.push(None);
                head_slots.push(s);
                head_from.push(match (origin.get(s).cloned().flatten(), own_origin.get(s).copied().flatten()) {
                    (Some((r, c)), _) => Some(HeadFrom::In(r, c)),
                    (None, Some((r, c))) => Some(HeadFrom::Own(r, c)),
                    _ => None,
                });
                arena.push(is_arena(s));
            }
            Term::Const(c) => {
                head.push(Some(c));
                head_slots.push(0);
                head_from.push(None);
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
                head_from.push(None);
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
    let est = stages.iter().map(|s| j.from.get(s.first + s.span.max(1) - 1).and_then(|&p| j.est.get(p)).copied().filter(|e| e.is_finite() && *e >= 0.0)).collect();
    Ok(Plan { stages, head, head_slots, head_from, arena, nslots, nsteps: j.steps.len(), key, est })
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
    step.mentions(x)
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
        Step::Eq { .. } | Step::Ne { .. } | Step::Let { .. } => true,
        Step::Func { name, bind: true, .. } if matches!(caps::device_fn(name), Some(caps::DeviceFn::Names { .. })) => true,
        Step::Test { op, p, matcher, .. } => (literal_test(op).is_some() || matcher.is_some()) && !me(p),
        Step::Cat { bind: true, .. } | Step::Func { name: "str_concat", bind: true, .. } => true,
        Step::Func { name, args, bind: true, .. } if caps::device_fn(name) == Some(caps::DeviceFn::RegexReplace) => args.len() == 3 && me(&args[0]) && !args[1..].iter().any(me),
        Step::Func { name, args, bind: true, .. } => text_function(name).is_some_and(|(_, nt, _)| nt > 0) && me(&args[0]) && !args[1..].iter().any(me),
        _ => false,
    }
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

/// Whether `rel` is a view of a source's rows chosen by constants alone (a statement's `WHERE c = v`), with no dead
/// rows, whose source the views' reads answer in place, every row of it: read there as a whole view is, the constants
/// the stage's keys past its own (`with_consts`, decision 0353 step 6), none uploaded.
fn reads_source(rel: &InRel) -> bool {
    rel.ndead == 0
        && rel.view.as_ref().is_some_and(|v| !v.whole() && v.by_constants() && v.src.device_view().is_some_and(|d| d.rows == v.src.len() && d.in_place.is_some()))
}

/// Column `c` of `rel` among its owner's columns: a whole view's (or one `reads_source` reads), its source's.
fn owner_col(rel: &InRel, c: usize) -> usize {
    match &rel.view {
        Some(v) if (v.whole() && rel.ndead == 0) || reads_source(rel) => v.column(c),
        _ => c,
    }
}

/// A stage's probe with the constants that chose its view's rows as keys past its own (`reads_source`): each
/// candidate is checked against them (the kernels' `matches`), its index of them too.
fn with_consts(mut d: mj::Stage, consts: &[(u32, i64)]) -> Result<mj::Stage, String> {
    for &(c, v) in consts {
        let i = d.nkeys as usize;
        if i >= mj::MAX_KEYS {
            return Err("a probe of a view chosen by constants with more keys than a stage holds".into());
        }
        d.key_kind[i] = mj::CONST;
        d.key_const[i] = v;
        d.key_slot[i] = 0;
        d.key_rcol[i] = c;
        d.nkeys += 1;
    }
    Ok(d)
}

/// The screen of a stage of `rel` (decision 0488): the kept filter of the first of `d`'s keys' columns (the
/// relation's own, before `narrowed`) whose sketch keeps one, where its sketches count every row (as the CPU's screen
/// borrows it: `join_filter`), bound where it lies in its block. `None` for a view's rows, or no such key.
fn screen_of(rel: &InRel, d: &mj::Stage) -> Result<Option<mj::Screen>, String> {
    if !crate::code_rule_dsl::logic::bloom::kept_keys_read() || !matches!(d.mode, mj::JOIN | mj::SEMI | mj::ANTI | mj::AGG | mj::WITNESS) {
        return Ok(None);
    }
    for k in 0..(d.nkeys as usize).min(mj::MAX_KEYS) {
        let Some(kept) = rel.kept_keys(d.key_rcol[k] as usize) else { continue };
        let b = kept.in_block();
        let device = rerail_accel::gpu::join::device()?;
        let held = rerail_accel::gpu::join::Held::kept(b.facts().block.device_buffer(device)?, Box::new(kept.clone()));
        return Ok(Some(mj::Screen { held: Rc::new(held), at: u32::try_from(b.at()).map_err(|_| "a filter past 4 GB of its block")?, key: k as u32 }));
    }
    Ok(None)
}

/// The columns of the relation an atom's stage reads (`d`'s keys, column tests, binds, row program and fold): the
/// device holds only these of it (decision 0277).
fn right_cols(d: &mj::Stage) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    out.extend(d.key_rcol[..d.nkeys as usize].iter().map(|&c| c as usize));
    for k in 0..d.ntests as usize {
        out.extend([d.test_a[k] as usize, d.test_b[k] as usize]);
    }
    out.extend((0..d.nbinds as usize).filter(|&k| d.bind_kind[k] == mj::BIND_RIGHT).map(|k| d.bind_idx[k] as usize));
    out.extend((0..d.ncols as usize).filter(|&k| d.col_kind[k] == mj::RIGHT).map(|k| d.col_idx[k] as usize));
    if d.mode == mj::AGG && d.agg[1] != mj::NONE {
        out.push(d.agg[1] as usize);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// `d` reading the relation's columns where the device holds them: column `c` at `at[c]` (`right_cols`' columns).
fn narrowed(mut d: mj::Stage, at: &[u32]) -> mj::Stage {
    let m = |c: u32| at[c as usize];
    for k in 0..d.nkeys as usize {
        d.key_rcol[k] = m(d.key_rcol[k]);
    }
    for k in 0..d.ntests as usize {
        (d.test_a[k], d.test_b[k]) = (m(d.test_a[k]), m(d.test_b[k]));
    }
    for k in 0..d.nbinds as usize {
        if d.bind_kind[k] == mj::BIND_RIGHT {
            d.bind_idx[k] = m(d.bind_idx[k]);
        }
    }
    for k in 0..d.ncols as usize {
        if d.col_kind[k] == mj::RIGHT {
            d.col_idx[k] = m(d.col_idx[k]);
        }
    }
    if d.mode == mj::AGG && d.agg[1] != mj::NONE {
        d.agg[1] = m(d.agg[1]);
    }
    d
}

thread_local! {
    static LAST: RefCell<Option<Report>> = const { RefCell::new(None) };
}

/// The waves flushed whose completion this thread keeps for a stratum still to resume.
#[cfg(test)]
fn waves_kept() -> usize {
    FLUSHED.with_borrow(HashMap::len)
}

/// Decision 0603's test, of every stratum the device runs in a test build: whatever its rules and rounds, its launches
/// are one program and its budget's relaunches (and its arenas' remaps, a parked claim's or text's), none a rerun's,
/// and its relaunches its items over a launch's budget. A breach panics with its report; `RERAIL_ONE_LAUNCH_CHECK=log`
/// prints one line a stratum instead, to count them. A device whose arenas cannot grow in place (an allocation of
/// their first bytes: `rerail_device::device::reserve_grows`) runs a stratum again past them, and is not checked.
fn one_launch(rep: &Report) {
    #[cfg(any(test, feature = "test-hooks"))]
    if rep.dispatches != 0 && rerail_accel::gpu::join::device().is_ok_and(rerail_device::device::reserve_grows) {
        // A stratum is one program; a wave's first stratum's report holds the wave's, one a stratum.
        let one = rep.waved != 0 || rep.programs == 1;
        let launches = rep.programs + rep.relaunches + rep.remaps;
        let budget = rep.launch_items.max(1);
        let ok = one && rep.dispatches == launches && rep.reruns == 0 && u64::from(rep.relaunches) <= rep.taken / budget + rep.programs as u64;
        if std::env::var_os("RERAIL_ONE_LAUNCH_CHECK").is_some_and(|v| v == "log") {
            eprintln!("ONE_LAUNCH {} programs {} reruns {} arena_reruns {} texts {} rounds {}", if ok { "ok" } else { "breach" }, rep.programs, rep.reruns, rep.arena_reruns, rep.made, rep.rounds);
        } else {
            assert!(ok, "a stratum past one launch and its budget's relaunches (decision 0603): {rep:?}");
        }
    }
    let _ = rep;
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
    /// Per atom stage, where its relation's columns lie in the device's copy (`resident`).
    places: Vec<Option<Vec<u32>>>,
    /// Each stage's constants that chose its view's rows, keys past its own (`reads_source`, `with_consts`).
    consts: Vec<Vec<(u32, i64)>>,
    /// Per witness stage, its probes' relations, sorts and descriptors (their columns where the copies hold them).
    probes: Vec<Option<Vec<(Rc<mj::DevRel>, Option<Rc<mj::DevIndex>>, mj::Stage)>>>,
    /// Each stage's screen: its relation's kept filter of a key's column (`screen_of`, decision 0488).
    screens: Vec<Option<mj::Screen>>,
    /// The head's distinct rows (a column per slot of `uniq_cols`), made on the device after the last stage; their
    /// count is `totals`' last word.
    uniq: Option<mj::Table>,
    uniq_cols: Vec<usize>,
    /// Its head's rows' key in its relation's first order (t58 increment A, `run_key`), its parts its `uniq_cols`'
    /// columns: they are then made distinct by sorting, in that order, not by a hash table.
    key: Option<mj::SortKey>,
    /// The text arena its text functions made their texts into (its batch's, shared with the batch's other jobs).
    arena: Option<Rc<mj::Arena>>,
}

/// `rel`'s rows where they lie in a block, and the columns there: a flat relation's (its dead rows marked by its
/// dead rows' delta, `InBlock::dead`, which the kernels skip), or a whole view's source's (the vtab's `Relation`).
fn placed(rel: &Arc<InRel>) -> Option<(crate::code_rule_dsl::facts::rows::InBlock, usize)> {
    match &rel.view {
        // A source made on lookup numbers only the rows looked up so far; the relation it is bound by holds every pair
        // (a layered axis's, its tape values' too, decision 0431 §8).
        Some(view) if view.whole() && rel.ndead == 0 => view.src.in_block().filter(|b| b.rows as usize == rel.len() || view.src.made_on_lookup()).map(|b| (b, view.src.columns().len())),
        _ => rel.in_block().map(|b| (b, rel.k)),
    }
}

/// The device's handle to `rel`'s columns `need` and `cols`, its index by `cols` (none when it has no keys), and
/// where each of its columns lies in the handle (`mj::NONE`: not held). Nothing is kept of it between strata
/// (decision 0478 steps 1 and 3): rows in a block, and a view read in place, are read where they lie, their indexes
/// found in their blocks (or built there by `batch`); a copy (rows uploaded, a view made on the device) is made for
/// this stratum, holding only the columns it reads, and its index built with it. A stratum in flight holds its own.
#[allow(clippy::type_complexity)]
///
/// A view of a source's rows chosen by constants (`reads_source`) is its source's view read in place, every row: the
/// constants come back with it, each its column in the handle and its value, keys past the stage's own; `probed`, a
/// stage probes it, so its index is by them too.
fn resident(rel: &Arc<InRel>, need: &[usize], cols: &[usize], probed: bool, batch: &mut mj::Batch, rep: &mut Report) -> Result<(Rc<mj::DevRel>, Option<Rc<mj::DevIndex>>, Vec<u32>, Vec<(u32, i64)>), String> {
    // Rows in a block are read where they lie (decision 0325 step 3).
    if let Some((b, ncols)) = placed(rel) {
        let dev = Rc::new(mj::DevRel::in_block(&b, ncols).map_err(|e| format!("{e:?}: a relation in its block"))?);
        rep.in_place += 1;
        // Every column of the owner, at its own place.
        let place: Vec<u32> = (0..rel.k).map(|c| owner_col(rel, c) as u32).collect();
        if cols.is_empty() {
            return Ok((dev, None, place, Vec::new()));
        }
        let by: Vec<usize> = cols.iter().map(|&c| place[c] as usize).collect();
        let ix = Rc::new(batch.index(&dev, &by).map_err(|e| format!("{e:?}: an index of {} rows", dev.rows))?);
        if ix.built { rep.sorts += 1 } else { rep.sort_hits += 1 }
        rep.moved += u32::from(ix.copied());
        return Ok((dev, Some(ix), place, Vec::new()));
    }
    // A `has` axis probed by one of its columns is read off the tape (decision 0353 step 5): a stage walks each
    // binding's rows there (`mj::Batch::by_has`), nothing made or uploaded. Read otherwise, its rows are uploaded.
    let walked = rel.ndead == 0 && matches!(cols.iter().map(|&c| owner_col(rel, c)).collect::<Vec<_>>()[..], [0] | [1]);
    if let Some(w) = rel.view.as_ref().filter(|_| walked).and_then(|v| v.src.has_walk()) {
        let dev = Rc::new(mj::DevRel::has(w.nodes, w.view, &w.kinds).map_err(|e| format!("{e:?}: a has axis read off the tape"))?);
        rep.in_place += 1;
        crate::code_rule_dsl::prof::count("device.view_read.has", 1);
        let place: Vec<u32> = (0..rel.k).map(|c| match owner_col(rel, c) { at @ (0 | 1) => at as u32, _ => mj::NONE }).collect();
        let by: Vec<usize> = cols.iter().map(|&c| place[c] as usize).collect();
        let ix = batch.by_has(&dev, &by).map_err(|e| format!("{e:?}: a has probe"))?.ok_or("a has axis probed by neither of its columns")?;
        return Ok((dev, Some(Rc::new(ix)), place, Vec::new()));
    }
    let c0 = thread_cpu();
    let whole = matches!(&rel.view, Some(v) if v.whole() && rel.ndead == 0);
    let filtered = reads_source(rel);
    // A view chosen by constants holds its source's every row (`reads_source`).
    let n_all = match &rel.view {
        Some(v) if filtered => v.src.len(),
        _ => rel.len(),
    };
    let ids: Option<Vec<usize>> = (rel.ndead > 0).then(|| (0..n_all).filter(|&id| !is_dead(&rel.dead, id as u32)).collect());
    let n = ids.as_ref().map_or(n_all, Vec::len);
    // A whole view its batch's chunk block makes (decision 0289 §2).
    let made = match &rel.view {
        Some(view) if whole => view.src.device_view().filter(|d| d.rows == n),
        // Read in place, never made (`reads_source`).
        Some(view) if filtered => view.src.device_view().filter(|d| d.rows == n && d.in_place.is_some()),
        _ => None,
    };
    let (dev, held): (mj::DevRel, Vec<usize>) = match made.as_ref().and_then(|d| Some((d.view, d.in_place.clone()?, d.home.clone()?))) {
        // A view the views' reads answer is read in place (decision 0353 step 4d): its relation of no columns, which
        // the view's source holds, keeps its indexes (decision 0478 step 3); every column is the view's own.
        Some((view, v, home)) => {
            let ncols = rel.view.as_ref().map_or(0, |view| view.src.columns().len());
            let dev = mj::DevRel::view_in(home, ncols, n, v).map_err(|e| format!("{e:?}: a view of {n} rows read in place"))?;
            rep.in_place += 1;
            // By the view's device code (`device_chunk::VIEW_*`), as `view_made` counts them.
            crate::code_rule_dsl::prof::count(["device.view_read.node", "device.view_read.child", "device.view_read.field", "device.view_read.core", "device.view_read.axis", "device.view_read.line", "device.view_read.end_line", "device.view_read.col", "device.view_read.docof", "device.view_read.tline", "device.view_read.tape", "device.view_read.depth", "device.view_read.order"].get(view as usize).copied().unwrap_or("device.view_read.other"), n);
            (dev, (0..ncols).collect())
        }
        None if filtered => return Err("a view chosen by constants whose source is no longer read in place".into()),
        None => {
            let mut want: Vec<usize> = need.iter().chain(cols).map(|&c| owner_col(rel, c)).collect();
            want.sort_unstable();
            want.dedup();
            let k = want.len();
            // The process's device budget short (the syntax side's tapes hold most of it): the pool's free buffers
            // give theirs back first.
            if mj::device_bytes_left() < 8 * k * n + headroom() {
                mj::give_back();
            }
            let dev = if let Some(d) = made {
// copy (0353 step 4): a view made on the device for a stratum (decision 0353).
                let device = rerail_accel::gpu::join::device()?;
                let block = d.block.device_buffer(device)?;
                let dev = mj::DevRel::empty(k, n).map_err(|e| format!("{e:?}: a view of {n} rows, {k} columns"))?;
                batch.expand_view(&block, d.root, d.view, &want, &dev).map_err(|e| format!("{e:?}: a view made on the device"))?;
                rep.uploads += 1;
                rep.upload_cpu += thread_cpu() - c0;
                let by = &mut rep.upload_by[profile::UPLOAD_MADE];
                // By view (`device_chunk::VIEW_*`), for the profile: what step 4 removes, view by view.
                crate::code_rule_dsl::prof::count(["device.view_made.node", "device.view_made.child", "device.view_made.field", "device.view_made.core", "device.view_made.axis", "device.view_made.line", "device.view_made.end_line", "device.view_made.col", "device.view_made.docof", "device.view_made.tline"].get(d.view as usize).copied().unwrap_or("device.view_made.other"), dev.bytes());
                by.0 += dev.bytes() as u64;
                by.1 += thread_cpu() - c0;
                dev
            } else {
                // What each upload read, and why it was not read where it lies: what step 6 removes, by view, and flat
                // rows no block holds as one relation with its dead rows' delta.
                {
                    let why = match &rel.view {
                        _ if ids.is_some() && rel.view.is_none() => "flat rows with dead rows, no block taking their delta",
                        None => "flat rows in no one block",
                        Some(_) if ids.is_some() => "dead rows",
                        Some(view) if !view.whole() && view.by_constants() => "filtered by constants",
                        Some(view) if !view.whole() => "of chosen rows (a version's, or picked)",
                        Some(view) => match view.src.in_block() {
                            Some(b) if b.rows as usize == n => "in a block, not taken",
                            Some(_) => "in a block of other rows",
                            None if view.src.device_view().is_some() => "made on the device, of other rows",
                            None => "in no block, not made on the device",
                        },
                    };
                    let cols: String = rel.view.as_ref().map_or_else(String::new, |v| v.src.columns().join(",")).chars().take(60).collect();
                    crate::code_rule_dsl::prof::count_named(format!("device.upload ({why}): {cols}"), 8 * k * n);
                }
// copy (0353 step 6): rows uploaded from the host for a stratum (decision 0353).
                let dev = mj::DevRel::upload(k, n, |v| {
                    let row_of = |r: usize| ids.as_ref().map_or(r, |ids| ids[r]);
                    match &rel.view {
                        Some(view) if whole => {
                            // The source's rows in order, the columns held read at once per row (`Source::for_rows`).
                            view.src.for_rows(&want, &mut |r, row| {
                                if r >= n {
                                    return;
                                }
                                for (c, &x) in row.iter().enumerate() {
                                    v[c * n + r] = x;
                                }
                            });
                        }
                        None => {
                            // Each wanted value read where it lies (`RowRef::Rows`), not the row copied whole.
                            for r in 0..n {
                                let row = rel.at(row_of(r));
                                for (c, &w) in want.iter().enumerate() {
                                    v[c * n + r] = row.at(w);
                                }
                            }
                        }
                        Some(_) => {
                            for r in 0..n {
                                let row = rel.at(row_of(r)).read();
                                for (c, &w) in want.iter().enumerate() {
                                    v[c * n + r] = row.at(w);
                                }
                            }
                        }
                    }
                })
                .map_err(|e| format!("{e:?}: an upload of {n} rows, {k} columns"))?;
                rep.uploads += 1;
                rep.upload_bytes += dev.bytes() as u64;
                rep.upload_cpu += thread_cpu() - c0;
                let by = &mut rep.upload_by[if rel.view.is_some() { profile::UPLOAD_VIEW } else { profile::UPLOAD_FLAT }];
                by.0 += dev.bytes() as u64;
                by.1 += thread_cpu() - c0;
                dev
            };
            (dev, want)
        }
    };
    let dev = Rc::new(dev);
    let place: Vec<u32> = (0..rel.k).map(|c| held.iter().position(|&w| w == owner_col(rel, c)).map_or(mj::NONE, |p| p as u32)).collect();
    // The constants that chose a view's rows, each its column in the handle: keys past the probe's own.
    let consts: Vec<(u32, i64)> = match &rel.view {
        Some(v) if filtered => v.eqs().iter().map(|&(c, x)| held.iter().position(|&w| w == c).map(|p| (p as u32, x)).ok_or("a constant's column not held")).collect::<Result<_, _>>()?,
        _ => Vec::new(),
    };
    // Held for its texts or verdicts (`probed` false), no stage probes it.
    if cols.is_empty() && (consts.is_empty() || !probed) {
        return Ok((dev, None, place, consts));
    }
    let own: Vec<usize> = cols.iter().map(|&c| place[c] as usize).collect();
    let by: Vec<usize> = own.iter().copied().chain(consts.iter().map(|&(c, _)| c as usize)).collect();
    // `child` probed by its parent: its children off the tape, no index of the view (decision 0353 step 4c); the
    // constants that chose its rows checked on each.
    if let Some(ix) = batch.by_parent(&dev, &own).map_err(|e| format!("{e:?}: a parent probe"))? {
        return Ok((dev, Some(Rc::new(ix)), place, consts));
    }
    let ix = Rc::new(batch.index(&dev, &by).map_err(|e| format!("{e:?}: an index of {} rows", dev.rows))?);
    // Found where a stratum before built it (a view's, in its block), or built here.
    if ix.built { rep.sorts += 1 } else { rep.sort_hits += 1 }
    rep.moved += u32::from(ix.copied());
    Ok((dev, Some(ix), place, consts))
}

/// Decision 0209 §6 on the device, with decision 0261: columns' statistics in one batch
/// (`rerail_accel::gpu::join::column_stats_of`). A relation in its block is read in place, its column where it
/// lies; any other's values are gathered on the host. Also how many columns were read in place.
pub fn column_stats(cols: &[StatColumn<'_>], k: usize) -> Result<(Vec<ColumnStats>, usize), String> {
    use rerail_accel::gpu::join::{StatCol, column_stats_of};
    let devs: Vec<Option<(Rc<mj::DevRel>, usize)>> = cols
        .iter()
        .map(|c| {
            let StatColumn::Rel(rel, col) = c else { return None };
            // A relation in its block: its column where it lies (decision 0478 step 1). Any other's is gathered.
            let (b, ncols) = placed(rel)?;
            let at = owner_col(rel, *col);
            (at < ncols).then(|| mj::DevRel::in_block(&b, ncols).ok().map(|d| (Rc::new(d), at))).flatten()
        })
        .collect();
    let gathered: Vec<Option<Vec<i64>>> = cols
        .iter()
        .zip(&devs)
        .map(|(c, d)| match (c, d) {
            (StatColumn::Rel(rel, col), None) => Some(column_values(rel, *col)),
            _ => None,
        })
        .collect();
    let stat: Vec<StatCol<'_>> = cols
        .iter()
        .zip(&devs)
        .zip(&gathered)
        .map(|((c, d), g)| match (c, d, g) {
            (StatColumn::Values(v), _, _) => StatCol::Host(v),
            // Every relation the device holds lies in a block (decision 0325 step 5), read there.
            (StatColumn::Rel(..), Some((d, at)), _) => StatCol::Device { buffer: &d.held().buffer, at: d.at as usize, rows: d.rows, col: Some(*at as u32) },
            (_, _, g) => StatCol::Host(g.as_deref().expect("gathered")),
        })
        .collect();
    let made = column_stats_of(&stat, k)?;
    let in_place = devs.iter().filter(|d| d.is_some()).count();
    Ok((made.into_iter().map(|m| ColumnStats { rows: m.rows, ndv: m.ndv, sq: m.sq, cube: m.cube, top: m.top }).collect(), in_place))
}

/// Column `col` of `rel`'s live rows, in row order (as the resident upload lays them out).
fn column_values(rel: &InRel, col: usize) -> Vec<i64> {
    let n = rel.len();
    let live = (0..n).filter(|&id| rel.ndead == 0 || !is_dead(&rel.dead, id as u32));
    if rel.view.is_none() {
        live.map(|id| rel.value(id, col)).collect()
    } else {
        live.map(|id| rel.at(id).read().at(col)).collect()
    }
}

/// Regex `regex`'s automata for its replacements on the device (`text::table::Replacer`), where the automata block
/// holds them (decision 0478 step 2c), and their states. Refused for a regex the device has no automata for.
fn automata(regex: &str) -> Result<(Rc<mj::DevBytes>, u32), String> {
    let r = crate::code_rule_dsl::text::table::replacer(regex).map_err(|e| format!("replace_all has no device form: {e}"))?;
    Ok((Rc::new(mj::DevBytes::of(&r.automaton).map_err(|e| format!("{e:?}"))?), r.states))
}

/// A `ROWTEXT` stage's automaton on the device and `Batch::row_stage`'s numbers for it ([its scratch's states, a
/// test table's states, classes, start]), where the automata block holds it.
fn row_automaton(auto: &RowAuto, names: &Interner) -> Result<Option<(Rc<mj::DevBytes>, [u32; 4])>, String> {
    Ok(match auto {
        RowAuto::None => None,
        RowAuto::Replace { pattern, with } => {
            if names.name(*with).contains('$') {
                return Err("replace_all with a replacement naming a group ($) has no device form".into());
            }
            let (a, states) = automata(&names.name(*pattern))?;
            Some((a, [states, 0, 0, 0]))
        }
        RowAuto::Test { table, .. } => {
            let b = Rc::new(mj::DevBytes::of(&table.automaton).map_err(|e| format!("{e:?}"))?);
            Some((b, [table.states, table.states, table.classes, table.start]))
        }
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
        let v = if rel.view.is_none() { rel.value(id, col) } else { rel.at(id).read().at(col) };
        if seen.insert(v) {
            ids.push(v);
        }
    }
    ids
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
                Kind::Filter | Kind::Atom { .. } | Kind::Own { .. } | Kind::Verdict { .. } | Kind::Names | Kind::RowText { .. } | Kind::Witness { .. } => {
                    io.push((slot, slot + 1));
                    slot += 1;
                }
            }
            if s.desc.mode == mj::AGG || reads_names(s.desc.mode) {
                // Its word: set when a sum passed 64 bits, a made text's part is past what a made value holds, or a
                // symbol's text is the host's (`TEXTFN_*`).
                counter.push(Some(ncounters));
                ncounters += 1;
            } else if s.npassed > 0 {
                counter.push(Some(ncounters));
                ncounters += 1 + s.npassed;
            } else {
                counter.push(None);
            }
        }
        let totals = mj::Words::zeroed(slot + 2).map_err(|e| format!("{e:?}: the totals of {} stages", slot + 2))?;
        totals.set(0, 1);
        let counters = mj::Words::zeroed(ncounters.max(1)).map_err(|e| format!("{e:?}: {} counters", ncounters.max(1)))?;
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
        Ok(Run { plan: Rc::new(plan.clone()), _plan: std::marker::PhantomData, io, tables, caps, totals, counters, counter, rels: vec![None; plan.stages.len()], places: vec![None; plan.stages.len()], consts: vec![Vec::new(); plan.stages.len()], screens: vec![None; plan.stages.len()], probes: vec![None; plan.stages.len()], uniq: None, uniq_cols, key: None, arena: None })
    }

    /// The space for stage `k`'s rows out, from its input's space.
    fn guess(&self, k: usize, cap_in: usize, nr: usize, own: &HashMap<(u64, usize), f64>) -> usize {
        let s = &self.plan.stages[k];
        match &s.kind {
            Kind::Atom { .. } | Kind::Own { .. } if s.desc.mode == mj::JOIN => {
                // The component's own rounds' growth, else the stage's as filed before its chunk (decision 0478 step 7).
                let seen = own.get(&(self.plan.key, k)).copied().or_else(|| crate::code_rule_dsl::logic::plan::joins::growth(self.plan.key, k));
                table_rows(cap_in, nr, s.desc.nkeys == 0 && cap_in == 1, seen, self.plan.est.get(k).copied().flatten())
            }
            _ => cap_in,
        }
    }

    /// The indexes stages `from..` probe, found again for `batch`, which encodes them again (`encode_levels`): an index
    /// the batch they were first encoded in built may have gone back when it was done (`mj::Batch::again`).
    fn indexes_again(&mut self, from: usize, batch: &mut mj::Batch) -> Result<(), String> {
        for k in from..self.plan.stages.len() {
            if let Some((dev, Some(ix))) = &mut self.rels[k] {
                *ix = batch.again(dev, ix).map_err(|e| format!("{e:?}: an index found again"))?;
            }
            for (dev, ix, _) in self.probes[k].iter_mut().flatten() {
                if let Some(ix) = ix {
                    *ix = batch.again(dev, ix).map_err(|e| format!("{e:?}: a probe's index found again"))?;
                }
            }
        }
        Ok(())
    }

    /// Encodes stage `k` into `batch`, queued in its level (`encode_levels`); the component's own relations are
    /// `own`'s.
    fn encode_stage(&mut self, k: usize, batch: &mut mj::Batch, rep: &mut Report, own: &mut Own, names: &Interner) -> Result<(), String> {
        {
            let s = &self.plan.stages[k];
            let (i, o) = self.io[k];
            match &s.kind {
                Kind::Pass => {}
                Kind::Map => {
                    let t = self.tables[i].as_ref().expect("the map's table");
                    batch.map(s.desc, t, i, &self.totals).map_err(|e| format!("{e:?}"))?;
                }
                Kind::Filter | Kind::Atom { .. } | Kind::Own { .. } | Kind::Verdict { .. } | Kind::Names | Kind::RowText { .. } | Kind::Witness { .. } => {
                    match (&s.kind, &self.rels[k]) {
                        (Kind::Atom { rel, cols }, None) => {
                            let (dev, ix, place, consts) = resident(rel, &right_cols(&s.desc), cols, true, batch, rep)?;
                            self.screens[k] = screen_of(rel, &s.desc)?;
                            self.rels[k] = Some((dev, ix));
                            self.places[k] = Some(place);
                            self.consts[k] = consts;
                        }
                        (Kind::Witness { rel, cols, rest }, None) => {
                            // The first atom's columns its stage reads and those the probes key on.
                            let mut need = right_cols(&s.desc);
                            for (_, _, b) in rest {
                                need.extend((0..b.nkeys as usize).filter(|&q| b.key_kind[q] == mj::ACOL).map(|q| b.key_slot[q] as usize));
                            }
                            let (dev, ix, place, consts) = resident(rel, &need, cols, true, batch, rep)?;
                            self.screens[k] = screen_of(rel, &s.desc)?;
                            let mut probes = Vec::new();
                            for (r, bcols, b) in rest {
                                let (bdev, bix, bplace, bconsts) = resident(r, &right_cols(b), bcols, true, batch, rep)?;
                                let mut b = with_consts(narrowed(*b, &bplace), &bconsts)?;
                                for q in 0..b.nkeys as usize {
                                    if b.key_kind[q] == mj::ACOL {
                                        b.key_slot[q] = place[b.key_slot[q] as usize];
                                    }
                                }
                                probes.push((bdev, bix, b));
                            }
                            self.rels[k] = Some((dev, ix));
                            self.places[k] = Some(place);
                            self.consts[k] = consts;
                            self.probes[k] = Some(probes);
                        }
                        (Kind::Own { rel, delta, cols }, None) => {
                            self.rels[k] = Some(own.get(*rel, *delta, cols, batch, rep)?.ok_or("an own relation with no rows")?);
                        }
                        _ => {}
                    }
                    let right = self.rels[k].as_ref().map(|(r, ix)| (&**r, ix.as_deref()));
                    let cap_in = self.caps[i];
                    if self.caps[o] == 0 {
                        // A view chosen by constants is read over its source's every row (`reads_source`): its rows
                        // are the view's own, which the constants choose. Sized by its source's, a scan's table held
                        // every row of `child` (the join scratch held at most rose from 93 to 141 MB, ab116). An own
                        // relation's rows are as many as it holds now (`Own::now`).
                        let nr = match &s.kind {
                            Kind::Atom { rel, .. } | Kind::Witness { rel, .. } if !self.consts[k].is_empty() => Some(rel.len()),
                            Kind::Own { rel, delta, .. } => own.now.get(&(*rel, *delta)).copied(),
                            _ => None,
                        };
                        self.caps[o] = self.guess(k, cap_in, nr.or(right.map(|r| r.0.rows)).unwrap_or(0), &own.growth);
                    }
                    let cap = self.caps[o];
                    if 8 * self.plan.nslots.max(1) * cap > table_max() {
                        return Err(format!("a binding table of {cap} rows past the device's budget"));
                    }
                    // Placed by its program (decision 0603 step 5): its rows' room the device's, from the rows its
                    // level counts; `cap` its rows as guessed.
                    let out = batch.table(self.plan.nslots, cap).map_err(|e| format!("{e:?}: a binding table of {cap} rows, {} slots", self.plan.nslots))?;
                    if let Some(c) = self.counter[k] {
                        for w in c..c + 1 + if s.desc.mode == mj::AGG || reads_names(s.desc.mode) { 0 } else { s.npassed } {
                            self.counters.set(w, 0);
                        }
                    }
                    if makes_text(s.desc.mode) && k == self.first_textfn().unwrap_or(usize::MAX) {
                        // Its texts, and those of the text functions after it, go into this batch's arena.
                        self.arena = Some(batch.arena().map_err(|e| format!("{e:?}"))?);
                    }
                    if reads_names(s.desc.mode) {
                        dev_atoms(batch, names)?;
                    }
                    let left = self.tables[i].as_ref().expect("the stage's input");
                    if s.desc.mode == mj::ROWTEXT {
                        // Its automaton, and its id in the one rolling cache: a test's (its verdicts) or the
                        // functions' (their texts).
                        let (auto, id) = match &s.kind {
                            Kind::RowText { auto, kept } => (row_automaton(auto, names)?, *kept),
                            Kind::Verdict { table, test } => {
                                let a = Rc::new(mj::DevBytes::of(&table.automaton).map_err(|e| format!("{e:?}"))?);
                                (Some((a, [table.states, table.states, table.classes, table.start])), *test)
                            }
                            _ => (None, 0),
                        };
                        batch.row_stage(s.desc, left, auto.as_ref().map(|(b, p)| (&**b, *p)), id, i, o, &self.totals, &self.counters, self.counter[k], &out)
                    } else {
                        let desc = with_consts(self.places[k].as_deref().map_or(s.desc, |at| narrowed(s.desc, at)), &self.consts[k])?;
                        if let Some(probes) = &self.probes[k] {
                            let rest: Vec<(mj::Stage, &mj::DevRel, Option<&mj::DevIndex>)> = probes.iter().map(|(r, ix, b)| (*b, &**r, ix.as_deref())).collect();
                            batch.witness(desc, left, right.ok_or("a witness with no relation")?, self.screens[k].as_ref(), &rest, i, o, &self.totals, &out)
                        } else {
                            batch.stage(desc, left, right, self.screens[k].as_ref(), i, o, &self.totals, &self.counters, self.counter[k], &out)
                        }
                    }
                    .map_err(|e| format!("{e:?}: a stage's space for {cap_in} rows"))?;
                    self.tables[o] = Some(out);
                }
            }
        }
        Ok(())
    }

    /// Encodes its head's rows made distinct, as the store keeps them (fewer rows back to the host), after its last
    /// stage.
    fn encode_head(&mut self, batch: &mut mj::Batch) -> Result<(), String> {
        let last = self.io.last().map_or(0, |io| io.1);
        let table = self.tables[last].as_ref().expect("the last table");
        let uniq = batch.table(self.uniq_cols.len(), table.cap).map_err(|e| format!("{e:?}: the head's table of {} rows", table.cap))?;
        match &self.key {
            Some(key) => batch.dedup_sorted(table, &self.uniq_cols, last, self.totals.len - 1, &self.totals, &uniq, None, key),
            None => batch.dedup(table, &self.uniq_cols, last, self.totals.len - 1, &self.totals, &uniq),
        }
        .map_err(|e| format!("{e:?}: the head's dedup over {} rows", table.cap))?;
        self.uniq = Some(uniq);
        Ok(())
    }

}

/// Encodes `runs` (each with its first stage) into `batch`, level-major (decision 0601): every run's stage at a depth
/// before any run's next, then their heads' dedups. A batch launches a level at a time (`mj::Batch::flush`), each
/// kernel once for its level's jobs, so a stratum's launches are its depths', not its rules'. The component's own
/// relations are `own`'s.
fn encode_levels(runs: &mut [(&mut Run<'_>, usize)], batch: &mut mj::Batch, rep: &mut Report, own: &mut Own, names: &Interner) -> Result<(), String> {
    let depth = runs.iter().map(|(r, _)| r.plan.stages.len()).max().unwrap_or(0);
    for k in 0..depth {
        for (r, from) in runs.iter_mut() {
            if k >= *from && k < r.plan.stages.len() {
                r.encode_stage(k, batch, rep, own, names)?;
            }
        }
    }
    for (r, _) in runs.iter_mut() {
        r.encode_head(batch)?;
    }
    Ok(())
}

impl<'p> Run<'p> {
    /// Its first text function's stage: a run that runs again in a new batch runs again from it (or before), its
    /// texts all in the new batch's arena.
    fn first_textfn(&self) -> Option<usize> {
        self.plan.stages.iter().position(|s| makes_text(s.desc.mode))
    }

    /// Whether its texts found the arena full and it could not grow (`mj::Arena::grow`): its jobs wanted past its room.
    fn arena_full(&self) -> bool {
        self.plan.stages.iter().any(|s| makes_text(s.desc.mode)) && self.arena.as_ref().is_some_and(|a| a.wanted() > a.cap())
    }

    /// Why its rows are not the CPU's, once it ran: an aggregate's sum past 64 bits (the CPU's fold holds 128).
    fn failed(&self) -> Option<String> {
        let set = |mode| self.plan.stages.iter().zip(&self.counter).any(|(s, c)| s.desc.mode == mode && c.is_some_and(|c| self.counters.get(c) != 0));
        if set(mj::AGG) {
            return Some("an aggregate's sum past 64 bits".to_owned());
        }
        if self.plan.stages.iter().zip(&self.counter).any(|(s, c)| makes_text(s.desc.mode) && c.is_some_and(|c| self.counters.get(c) & mj::TEXTFN_HOST != 0))
        {
            return Some("lower of a name past ASCII (Unicode's mapping is the host's)".to_owned());
        }
        if self.arena_full() {
            return Some(format!("its texts past the device's text arena of {} bytes", arena_max()));
        }
        let bit = |b: u32| self.plan.stages.iter().zip(&self.counter).any(|(s, c)| reads_names(s.desc.mode) && c.is_some_and(|c| self.counters.get(c) & b != 0));
        if bit(mj::TEXTFN_UNHELD) {
            return Some("a text read of a symbol the atoms directory does not hold".to_owned());
        }
        if self.plan.stages.iter().zip(&self.counter).any(|(s, c)| s.desc.mode == mj::TEXTOP && c.is_some_and(|c| self.counters.get(c) & mj::TEXTFN_HOST != 0)) {
            return Some("a text test of a bound pattern holding the set separator (a set of patterns)".to_owned());
        }
        // A 16z view's blocks: a read that found no room in its batch's cache read the dead entry (0353 step 4b).
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        if self.rels.iter().flatten().filter_map(|(r, _)| r.view.as_ref()).any(|v| v.cache_failed() > v.failed) {
            return Some("a 16z block found no room in its batch's cache".to_owned());
        }
        None
    }

    /// The device bytes of its tables.
    fn bytes(&self) -> usize {
        self.tables.iter().chain(std::iter::once(&self.uniq)).flatten().map(mj::Table::bytes).sum()
    }

    /// The first stage whose rows overflowed its table, with its true count; the counts of every stage before are
    /// final. A placed table's room is its rows (decision 0603 step 5): never past, before its batch is done or after
    /// (`Table::device_sized`; `placed` is false once it is done, and its count is then past the host's guess).
    fn overflow(&self) -> Option<(usize, usize)> {
        self.plan.stages.iter().enumerate().find_map(|(k, _)| {
            let (i, o) = self.io[k];
            if i == o || self.tables[o].as_ref().is_some_and(mj::Table::device_sized) {
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
pub(in crate::code_rule_dsl) fn run(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    run_(rules, store, names, fresh)
}

fn run_(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
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

/// The rules that can make rows (none of them reads an empty input, `reads_empty_input`), by their places.
fn live(rules: &[(&Plan, &JoinRule, usize)]) -> Vec<usize> {
    (0..rules.len()).filter(|&k| !super::reads_empty_input(rules[k].1)).collect()
}

/// Rows per rule: the live rules' (`live`) at their places, none for the others.
fn spliced(n: usize, live: &[usize], rows: Vec<u64>) -> Vec<u64> {
    let mut out = vec![0; n];
    for (&k, r) in live.iter().zip(rows) {
        out[k] = r;
    }
    out
}

fn run_here(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    // A rule over an empty input makes no row, and its steps see none: it is not encoded, as the CPU does not walk it.
    let live = live(rules);
    if live.len() < rules.len() {
        let some: Vec<(&Plan, &JoinRule, usize)> = live.iter().map(|&k| rules[k]).collect();
        return run_here(&some, store, names, fresh).map(|rows| spliced(rules.len(), &live, rows));
    }
    let _in = in_device();
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
                mj::give_back();
                cleared = true;
            }
            Err(e) => return Err(e),
        }
    }
    rep.encode_cpu = (thread_cpu() - c0 - rep.wait_cpu - rep.decode_cpu - rep.upload_cpu).max(0.0);
    let rows: u64 = rep.rows;
    crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "head rows read back", rows);
    one_launch(&rep);
    LAST.with_borrow_mut(|l| *l = Some(rep));
    Ok(out)
}

thread_local! {
    /// The waves flushed, by number: their completion (or why they failed), and the device's seconds not yet
    /// counted by a stratum.
    /// With the bytes its programs' claims wanted where they passed their arena (0: none), for each of its strata.
    static FLUSHED: RefCell<HashMap<u64, Result<(Option<mj::Done>, usize, bool), String>>> = RefCell::new(HashMap::new());
    /// Strata waiting for their wave, by ticket: their encoded runs, their wave, their report so far.
    static SUSPENDED: RefCell<HashMap<u64, (Vec<Run<'static>>, u64, Report, Vec<usize>, Option<PrePlaced>)>> = RefCell::new(HashMap::new());
    /// How many device strata this thread is inside (`InDevice`): nested when a wait ran another job's.
    static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A device stratum under way on this thread (`run_here`, `submit_`, `resume_`, `fixpoint_here`). When the outermost
/// one ends and no stratum of the thread waits for its wave, the thread's join scratch is given back
/// (`join::scratch_return`, Q203): to the device, which frees it once the command buffers that hold it completed. The
/// next stratum's sorts lease theirs anew.
///
/// It is the stratum's evaluation of the launch budget (`rerail_accel::gpu::launch`), or the caller's when it is in
/// one: every launch of its joins, of its rounds and of its stages run again (those a rerun's, decision 0540) is
/// charged to it, and past it is the hard failure (decision 0601).
struct InDevice(#[allow(dead_code, reason = "held: the stratum's evaluation")] rerail_accel::gpu::launch::LaunchScope);

fn in_device() -> InDevice {
    DEPTH.with(|d| d.set(d.get() + 1));
    InDevice(rerail_accel::gpu::launch::LaunchScope::enter())
}

impl Drop for InDevice {
    fn drop(&mut self) {
        let outer = DEPTH.with(|d| {
            d.set(d.get() - 1);
            d.get() == 0
        });
        if outer && SUSPENDED.with_borrow(HashMap::is_empty) {
            let n = rerail_accel::gpu::join::scratch_return();
            if n > 0 {
                crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "join scratch given back (MB)", (n >> 20) as u64);
            }
        }
    }
}

/// The waves' and the tickets' numbers, of the process. A parked stratum can resume on another thread than the one it
/// was submitted on (a job run while its thread waited, which joined a wave that thread had open, `plan::issue`). That
/// thread's `SUSPENDED` must then miss it, and a miss refuses the stratum to the CPU. With numbers of one thread, it
/// found that thread's own stratum of the same number instead, and resumed it with these rules: a panic on a rule
/// index, or another stratum's rows.
static WAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TICKETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A batch of its own (not the wave's), with a text arena of `arena` bytes when given. The thread's open wave is
/// committed first: the relations kept resident (`resident`, `verdicts`, the texts) take their sorts, indexes and
/// verdicts from the batch that first asked for them, often the wave, and the cache hands them out at once. A batch
/// committed before the wave would read them unwritten (Q48: a stratum run apart, beside a wave it did not join,
/// read its inputs' sorts empty). The queue is one, so the wave's work, committed first, runs first.
fn direct_batch(arena: Option<usize>) -> Result<mj::Batch, String> {
    commit_wave()?;
    match arena {
        Some(cap) => mj::Batch::with_arena(cap),
        None => mj::Batch::new(),
    }
    .map_err(|e| format!("{e:?}"))
}

/// A batch of its own over `arena`, a recursive component's text arena (decision 0307), its rounds' texts going into
/// it; the thread's open wave committed first, as `direct_batch`'s (Q48).
fn shared_batch(arena: &Rc<mj::Arena>) -> Result<mj::Batch, String> {
    commit_wave()?;
    mj::Batch::with_shared_arena(Rc::clone(arena)).map_err(|e| format!("{e:?}"))
}

/// The level's wave (decision 0113 §3): the strata placed on the device encode their joins into its batch and wait
/// for it, so a level's strata wait once. Its number and its batch. It is the job's open submission
/// (`lanes::wave::with_open`, decision 0386), not its thread's: a job nested on a thread opens a wave of its own, and
/// the wave is finished by the job that opened it.
type Wave = (u64, mj::Batch);

/// The job's open wave, committed (`direct_batch`). Its batch holds no open command buffer between strata (`submit`
/// commits before it returns, decision 0386): this commits only what a stratum's own encoding left.
fn commit_wave() -> Result<(), String> {
    crate::code_rule_dsl::logic::lanes::wave::with_open::<Wave, _>(|w| match w {
        Some((_, wave)) => wave.commit_open().map_err(|e| format!("{e:?}")),
        None => Ok(()),
    })
}

/// A stratum's joins encoded into the level's open command buffer, to wait for the wave (`lanes::wave::flush`
/// commits it once, for every stratum in it): its ticket for `resume`. `None` when they do not all fit one
/// submission's space (the stratum then runs on its own, `run`).
pub(in crate::code_rule_dsl) fn submit(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner) -> Result<Option<u64>, String> {
    submit_(rules, store, names)
}

fn submit_(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner) -> Result<Option<u64>, String> {
    let _in = in_device();
    let _span = crate::code_rule_dsl::logic::profile::span(|| "device joins (to the wave)".into());
    // A wave holds its strata's buffers to the level's end: with the budget's headroom gone (the wave's own leases
    // count, `device_bytes_left`), this stratum runs in a submission of its own, whose buffers go back when it ends.
    if mj::device_bytes_left() < headroom() {
        crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "strata run apart: the wave's budget was spent", 1);
        return Ok(None);
    }
    // As `run_here`: a rule over an empty input is not encoded; `resume` gives it no rows.
    let live = live(rules);
    let some: Vec<(&Plan, &JoinRule, usize)> = live.iter().map(|&k| rules[k]).collect();
    let rules = &some[..];
    let c0 = thread_cpu();
    let mut rep = Report { rules: rules.len() as u32, stages: rules.iter().map(|r| r.0.stages.len() as u32).sum(), ..Report::default() };
    // Its relations' rows had before the wave is borrowed: the encode reads their lengths inside it (`resident`), and a
    // relation the planner measured on a sample (decision 0494's `MadeRows::sampled`) makes its documents then, on the
    // pool, whose jobs enter wave scopes of their own on this thread (`lanes::wave::enter`: the borrow held, a panic).
    // Before its room is reserved, too: made holding it, the batch's blocks kept other strata short (ab190:
    // `ruff_safe_operand` refused, 193 MB short, 1.1 s on the CPU).
    made_before_encoding(rules);
    // Its tables' room, reserved whole before anything is encoded: a wait for it holds no command buffer (0386).
    let _room = admitted(rules, &mut rep)?;
    use crate::code_rule_dsl::logic::lanes::wave::with_open;
    // A process that did not make the pool (`threads::configure`: a test's) gets the job scopes at its first wave.
    crate::code_rule_dsl::logic::lanes::wave::install_job_scope();
    let opened = with_open::<Wave, _>(|w| w.is_none());
    if opened {
        let n = WAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        with_open::<Wave, _>(|w| -> Result<(), String> {
            // Its arena address space mapped as its claims reach it, as `group`'s: a claim past its pages parks and
            // the program goes on, never a rerun. Host-read, as its strata's heads may be read on the host.
            let mut batch = mj::Batch::new().map_err(|e| format!("{e:?}"))?;
            batch.arena_reserved(true);
            *w = Some((n, batch));
            Ok(())
        })?;
        crate::code_rule_dsl::logic::lanes::wave::defer(Box::new(move || {
            let Some((n, mut batch)) = with_open::<Wave, _>(Option::take) else { return Ok(()) };
            // Its strata's launches were charged as each encoded (`in_device`); what it commits here is checked in an
            // evaluation of its own.
            let _evaluation = rerail_accel::gpu::launch::LaunchScope::enter();
            let done = on_lane("stratum joins (a wave)", 1, || batch.finish().map_err(|e| format!("{e:?}")));
            // A failed wave fails its strata, which then run on the CPU: the flush itself goes on.
            // Kept for the strata that wait on it; a wave none waits on (its work held, unread) keeps nothing.
            if SUSPENDED.with_borrow(|s| s.values().any(|v| v.1 == n)) {
                FLUSHED.with_borrow_mut(|f| f.insert(n, done.map(|d| {
                    let full = if d.arena_full { d.arena_wanted.max(1) } else { 0 };
                    let stopped = d.stopped;
                    (Some(d), full, stopped)
                })));
            }
            Ok(())
        }));
    }
    // Encoded and committed before it returns (decision 0386): the wave's batch holds its strata's committed command
    // buffers, waited for once when the wave is flushed, and no open one. An open command buffer outliving this call
    // would be held by a thread that then waits, or goes idle: past the queue's 64, every thread asking for one waits.
    let encoded = with_open::<Wave, _>(|w| {
        let (n, batch) = w.as_mut().expect("the open wave");
        let runs = encode_runs(rules, batch, &mut rep, names, store);
        // Its heads' rows placed by its own program (`encode_place`), as `run`'s; its rows' rooms fresh, as `resume`
        // finds them.
        let fresh: Vec<Fresh> = vec![Fresh::default(); store.rels.len()];
        let pre = match &runs {
            Ok(r) if r.len() == rules.len() => encode_place(r, rules, store, &fresh, &mut rep, batch),
            _ => Ok(None),
        };
        let committed = batch.commit_open().map_err(|e| format!("{e:?}"));
        runs.and_then(|runs| pre.and_then(|pre| committed.map(|()| (*n, runs, pre))))
    })?;
    let (wave, runs, pre) = encoded;
    if runs.len() < rules.len() {
        // Encoded work it will not wait for: the wave runs it and nothing reads it, but it writes the runs' buffers.
        // They go now: the wave's submissions' fences hold them until that work completes (decision 0491 step 2), so
        // none goes back to the pool while the GPU writes it.
        drop(runs);
        return Ok(None);
    }
    rep.encode_cpu = thread_cpu() - c0 - rep.upload_cpu;
    let t = TICKETS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    SUSPENDED.with_borrow_mut(|s| s.insert(t, (runs, wave, rep, live, pre)));
    // Its stratum resumes on this thread, which holds its runs and its wave (`lanes::wave::kept_here`).
    crate::code_rule_dsl::logic::lanes::wave::joined();
    Ok(Some(t))
}

/// A stratum that waited for its wave (`submit`'s `ticket`), finished: as `run`, from the flushed submission.
pub(in crate::code_rule_dsl) fn resume(ticket: u64, rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    resume_(ticket, rules, store, names, fresh)
}

fn resume_(ticket: u64, rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    let _in = in_device();
    let _span = crate::code_rule_dsl::logic::profile::span(|| "device joins (after the wave)".into());
    let Some((runs, wave, mut rep, live, pre)) = SUSPENDED.with_borrow_mut(|s| s.remove(&ticket)) else { return Err("a stratum's device joins were not submitted".into()) };
    let all = rules.len();
    let some: Vec<(&Plan, &JoinRule, usize)> = live.iter().map(|&k| rules[k]).collect();
    let rules = &some[..];
    let done = FLUSHED.with_borrow_mut(|f| match f.get_mut(&wave) {
        None => Err("a stratum's wave was not flushed".to_owned()),
        Some(Err(e)) => Err(e.clone()),
        // The wave's device seconds, counted once (by its first stratum); its arena's state, every stratum's.
        Some(Ok((d, full, stopped))) => Ok((d.take(), *full, *stopped)),
    });
    // The wave's last stratum to resume lets its entry go: a wave's strata all wait on the thread that opened it, and
    // none joins it once flushed.
    if !SUSPENDED.with_borrow(|s| s.values().any(|v| v.1 == wave)) {
        FLUSHED.with_borrow_mut(|f| f.remove(&wave));
    }
    let (done, full, stopped) = done?;
    if let Some(d) = done {
        tally(&mut rep, &d);
    }
    rep.waved = 1;
    let c0 = thread_cpu();
    // A gate that stopped a program of the wave (a head's rows past their room): its heads placed again, by `place`.
    let out = complete(runs, rules, store, names, fresh, &mut rep, full, pre.filter(|_| !stopped))?;
    rep.encode_cpu += (thread_cpu() - c0 - rep.decode_cpu).max(0.0);
    let rows: u64 = rep.rows;
    crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "head rows read back", rows);
    one_launch(&rep);
    LAST.with_borrow_mut(|l| *l = Some(rep));
    Ok(spliced(all, &live, out))
}

/// The first rules of `rules` that fit one submission's space, run: their head rows as `run`'s.
fn group(rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh], rep: &mut Report) -> Result<Vec<u64>, String> {
    let room = admitted(rules, rep)?;
    let mut batch = direct_batch(None)?;
    let runs = encode_runs(rules, &mut batch, rep, names, store)?;
    // Its heads' rows placed in the same program (decision 0603: a stratum is one launch), from the device's counts.
    let pre = encode_place(&runs, rules, store, fresh, rep, &mut batch)?;
    // Its arena is address space mapped as its claims reach it (`mj::Batch::arena_reserved`): a claim past it maps
    // more, and nothing runs again. Where the host reads a placed table of it (a head read back, every head not placed
    // by the program), one the host reads (on Metal a wrapped one, not private storage).
    let placed_all = pre.as_ref().is_some_and(|p| p.heads.len() == runs.iter().zip(rules).map(|(_, r)| r.2).collect::<std::collections::BTreeSet<_>>().len());
    batch.arena_reserved(!placed_all);
    drop(room);
    let (t0, w0) = (Instant::now(), thread_cpu());
    let done = batch.finish().map_err(|e| format!("{e:?}"))?;
    rep.wait_wall += t0.elapsed().as_secs_f64();
    rep.wait_cpu += thread_cpu() - w0;
    tally(rep, &done);
    let full = if done.arena_full { done.arena_wanted.max(1) } else { 0 };
    complete(runs, rules, store, names, fresh, rep, full, pre.filter(|_| !done.stopped))
}

/// The first rules of `rules` that fit one submission's space, each encoded into `batch`.
/// Each relation the rules' stages read, made as `resident` reads it (`len`, and a view's source's), outside the wave.
fn made_before_encoding(rules: &[(&Plan, &JoinRule, usize)]) {
    let made = |rel: &Arc<InRel>| {
        if let Some(v) = &rel.view {
            v.src.len();
        }
        rel.len();
    };
    for (plan, _, _) in rules {
        for s in &plan.stages {
            match &s.kind {
                Kind::Atom { rel, .. } => made(rel),
                Kind::Witness { rel, rest, .. } => {
                    made(rel);
                    rest.iter().for_each(|(r, _, _)| made(r));
                }
                _ => {}
            }
        }
    }
}

fn encode_runs(rules: &[(&Plan, &JoinRule, usize)], batch: &mut mj::Batch, rep: &mut Report, names: &Interner, store: &Store) -> Result<Vec<Run<'static>>, String> {
    // A lease past the Metal budget is refused here (decision 0126): the stratum then runs on the CPU, named.
    rerail_accel::gpu::shared::within_budget(|| encode_runs_bounded(rules, batch, rep, names, store))
}

fn encode_runs_bounded(rules: &[(&Plan, &JoinRule, usize)], batch: &mut mj::Batch, rep: &mut Report, names: &Interner, store: &Store) -> Result<Vec<Run<'static>>, String> {
    let _h = crate::code_rule_dsl::logic::lanes::prof::host("device join (decision 0095)", "encode: uploads, sorts, stages");
    // The rules whose tables' expected bytes (`Plan::expected`, as `admitted` reserves them) fit one submission's
    // space, encoded together, level-major: their stages at each depth launch together (decision 0601).
    let (mut take, mut bytes) = (0, 0);
    for (plan, _, _) in rules {
        if take > 0 && (bytes > batch_max() || mj::device_bytes_left() < headroom()) {
            break;
        }
        bytes += plan.expected().0;
        take += 1;
    }
    let mut runs = rules[..take].iter().map(|(plan, _, _)| Run::new(plan)).collect::<Result<Vec<Run<'static>>, String>>()?;
    // Each head's rows made distinct in its first order where its bounds pack it (t58 increment A).
    let bounds = head_bounds(&rules[..take], store);
    for (r, (plan, _, h)) in runs.iter_mut().zip(rules) {
        r.key = bounds.get(h).and_then(|b| run_key(plan, &r.uniq_cols, &store.rels[*h], b));
    }
    // On a refusal the runs go, with the stages encoded: the passes that reach their buffers hold them until their
    // work completes (decision 0491 step 2).
    encode_levels(&mut runs.iter_mut().map(|r| (r, 0)).collect::<Vec<_>>(), batch, rep, &mut Own::default(), names)?;
    Ok(runs)
}

/// Commits `batch` and waits for it, the wait and the device's seconds into `rep`. The bytes its programs' claims
/// wanted where they passed their arena (`Done::arena_full`), else 0.
fn finish_timed(batch: &mut mj::Batch, rep: &mut Report) -> Result<usize, String> {
    let (t0, w0) = (Instant::now(), thread_cpu());
    let done = batch.finish().map_err(|e| format!("{e:?}"))?;
    rep.wait_wall += t0.elapsed().as_secs_f64();
    rep.wait_cpu += thread_cpu() - w0;
    tally(rep, &done);
    Ok(if done.arena_full { done.arena_wanted.max(1) } else { 0 })
}

/// Runs whose submission completed, finished: an overflowing stage run again from its exact count (in a batch of
/// their own), then the rows into the store (`fresh`) and each step's counts. `rules` are the runs' rules, in order.
/// `full`: the bytes its program's claims wanted where they passed its arena (decision 0603 step 5): every run again
/// from its first stage, in an arena of twice that.
/// `pre`: its heads' rows as its own program placed them (`encode_place`), taken where no run ran again (else placed
/// again, from the rows the runs made then).
#[allow(clippy::too_many_arguments)]
fn complete(mut runs: Vec<Run<'static>>, rules: &[(&Plan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh], rep: &mut Report, mut full: usize,
            pre: Option<PrePlaced>) -> Result<Vec<u64>, String> {
    let reruns0 = rep.reruns;
    let mut pending: Vec<usize> = (0..runs.len()).collect();
    let mut batch: Option<mj::Batch> = None;
    loop {
        if full > 0 {
            rep.reruns += pending.len() as u32;
            let b = match &mut batch {
                Some(b) => b,
                None => batch.insert(direct_batch(None)?),
            };
            b.arena_floor(2 * full);
            let _rerun = rerail_accel::gpu::launch::rerun();
            for &k in &pending {
                runs[k].indexes_again(0, b)?;
            }
            let mut todo: Vec<(&mut Run<'static>, usize)> = runs.iter_mut().enumerate().filter(|(k, _)| pending.contains(k)).map(|(_, r)| (r, 0)).collect();
            encode_levels(&mut todo, b, rep, &mut Own::default(), names)?;
            full = finish_timed(b, rep)?;
            continue;
        }
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
            None => batch.insert(direct_batch(Some(arena.max(mj::ARENA_START)))?),
        };
        // A run again is a rerun (decision 0540): its launches charge the run's reruns' budget.
        let _rerun = rerail_accel::gpu::launch::rerun();
        for &(k, s) in &again {
            runs[k].indexes_again(s, b)?;
        }
        let mut todo: Vec<(&mut Run<'static>, usize)> =
            runs.iter_mut().enumerate().filter_map(|(k, r)| again.iter().find(|a| a.0 == k).map(|&(_, s)| (r, s))).collect();
        encode_levels(&mut todo, b, rep, &mut Own::default(), names)?;
        full = finish_timed(b, rep)?;
    }
    // Each head's rows written in place, in a relation of a block, which the store takes as they lie (decision 0325
    // step 4); the heads `place` cannot take read back below.
    let placed = match pre {
        Some(pre) if rep.reruns == reruns0 => pre.claim(rep),
        _ => place(&runs, rules, store, fresh, rep)?,
    };
    // The rows into the store, and each step's counts.
    let d0 = thread_cpu();
    let _h = crate::code_rule_dsl::logic::lanes::prof::host("device join (decision 0095)", "decode: rows into the store");
    let mut out = Vec::with_capacity(runs.len());
    for (r, &(plan, j, head)) in runs.iter().zip(rules) {
        let last = r.io.last().map_or(0, |io| io.1);
        let n = r.totals.get(last) as usize;
        let distinct = r.totals.get(r.totals.len - 1) as usize;
        if placed.contains(&head) {
            out.push(n as u64);
            rep.rows += n as u64;
            let _ = counted(r, plan, j, rep);
            continue;
        }
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
            let ids = names.atoms().intern_all(&strs).map_err(|e| format!("the arena texts' symbols: {e}"))?;
            rep.made += strs.len() as u64;
            Ok(Some(vals.into_iter().zip(ids).map(|(v, id)| (v, i64::from(id))).collect()))
        }).collect::<Result<_, _>>()?;
        // Each column's values by row, read in step (a constant head's column is not read).
        let mut values: Vec<_> = cols.iter().map(|col| col.iter().copied()).collect();
        for _ in 0..distinct {
            for (c, h) in plan.head.iter().enumerate() {
                row[c] = h.unwrap_or_else(|| values[c].next().expect("a value at each distinct row"));
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
        let _ = counted(r, plan, j, rep);
    }
    drop(_h);
    rep.decode_cpu += thread_cpu() - d0;
    // The placed heads' rows into the store: their parts, as the device wrote them.
    for (h, part, sorted) in placed.into_parts() {
        if sorted { fresh[h].place_in_order(part) } else { fresh[h].place(part, None) }
    }
    Ok(out)
}

/// The heads whose rows `place` wrote in place, and where it wrote them.
#[derive(Default)]
struct PlacedHeads {
    /// Each head's rows placed, and whether they are in its first order (t58 increment A: the host takes them so).
    parts: Vec<(usize, super::Placed, bool)>,
    heads: Vec<usize>,
}

impl PlacedHeads {
    fn contains(&self, head: &usize) -> bool {
        self.heads.contains(head)
    }
    fn into_parts(self) -> Vec<(usize, super::Placed, bool)> {
        self.parts
    }
}

/// Where the device writes `n` new rows of `rel` (`k` columns), whose rows written so far and not yet taken are
/// `fresh`'s: after the last of those (a stratum's earlier group's, for the same head), else past its tail's rows, in
/// room reserved in their block (decision 0384 step 3: the relation stays one, and a reader binds it whole), else a
/// relation of their own (`Part::for_writes`) that becomes its tail. The rows' place, and the relation bound for the
/// writes.
fn head_room(rel: &super::Rel, fresh: &super::Fresh, k: usize, n: usize) -> Result<(super::Placed, mj::DevRel, usize), String> {
    // A block with no room for them: the relation's rows moved into one with room (`Part::moved_room`), which the
    // claim makes its tail, so it stays one relation.
    let moved = |l: Option<crate::code_rule_dsl::facts::rows::Lent>, why: &'static str| l.inspect(|_| crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", why, 1));
    let room = match fresh.last_placed() {
        Some(super::Placed::Part(p)) => p.room(n).or_else(|| moved(Some(p.moved_room(p.len(), n)), "heads' relations moved to room for a next lot")),
        Some(super::Placed::Tail { lent, n: written }) => lent.after(*written, n).or_else(|| moved(Some(lent.moved(*written, n)), "heads' relations moved to room for a next lot")),
        None => rel.tail_room(n).or_else(|| moved(rel.tail_moved(n), "heads' tails moved to room for the device's rows")),
    };
    if let Some(lent) = room {
        let at = lent.at;
        let dst = mj::DevRel::in_block(&lent.tail().in_block(at + n as u32), k).map_err(|e| format!("{e:?}: a head's tail in its block"))?;
        crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "heads' rows written into their own tail", 1);
        return Ok((super::Placed::Tail { lent: std::sync::Arc::new(lent), n: 0 }, dst, at as usize));
    }
    crate::code_rule_dsl::logic::lanes::prof::count("device join (decision 0095)", "heads' rows written into a relation of their own (no tail with rows)", 1);
    // A large one alone in a block with room past its rows for the indexes its readers probe it by (decision 0353
    // step 6): in the run's shared block, a later probe found no room there and copied it (`DevIndex::moved`, 766.4
    // MB in 247 copies on the M3's 300-file run, ab110). Room for a full directory of them (`ADDED_INDEXES`): with
    // room for one a column and one more, a relation of three columns probed by more keys was copied (ab118). Its
    // block is memory only as it is written. A small one shares the run's block, its copy as small.
    let part = if n >= ALONE_ROWS { Part::with_index_room(crate::code_rule_dsl::facts::rows::Life::Run, k, n, rerail_blocks::relation::ADDED_INDEXES) } else { Part::for_writes(k, n) };
    let dst = mj::DevRel::in_block(&part.in_block(n as u32), k).map_err(|e| format!("{e:?}: a head's rows in their block"))?;
    Ok((super::Placed::Part(part), dst, 0))
}

/// The rows from which a head's relation the device writes is alone in a block with room for its indexes
/// (`head_room`).
const ALONE_ROWS: usize = 4096;

/// `room` (`head_room`'s) with its `n` rows written: the rows the store takes.
///
/// # Safety
/// The batch that wrote them is finished, and wrote every value of rows `0..n` of the room (`n` within it).
unsafe fn written(room: super::Placed, n: usize) -> super::Placed {
    match room {
        // SAFETY: the caller's.
        super::Placed::Part(part) => {
            unsafe { part.written(n as u32) };
            super::Placed::Part(part)
        }
        super::Placed::Tail { lent, .. } => super::Placed::Tail { lent, n: n as u32 },
    }
}

/// The heads' rows a stratum's program placed (`encode_place`): each head's room, its count's word and slot, and its
/// bound; and what the work read, held until the program is done.
struct PrePlaced {
    heads: Vec<usize>,
    /// Each head's room, its count's slot and words, and whether its rows are in its first order (t58 increment A).
    pending: Vec<(usize, super::Placed, usize, mj::Words, bool)>,
    _hold: (Vec<mj::Table>, Vec<(mj::DevRel, mj::DevIndex)>),
}

impl PrePlaced {
    /// Its heads' rows as the finished program wrote them.
    fn claim(self, rep: &mut Report) -> PlacedHeads {
        let mut placed = PlacedHeads { heads: self.heads, parts: Vec::new() };
        for (h, room, slot, words, sorted) in self.pending {
            let n = words.get(slot) as usize;
            // SAFETY: the program is finished, and its gate held each head's rows within the room `head_room`
            // reserved; `place_rows` wrote every column of rows `0..n`.
            let part = unsafe { written(room, n) };
            rep.distinct += n as u64;
            rep.placed += n as u64;
            placed.parts.push((h, part, sorted));
        }
        placed
    }
}

/// The bytes of a head's room past which `encode_place` leaves the head to `place`: a block's room is real memory on
/// Metal once bound.
const PLACE_ROOM_BYTES: usize = REACH_BYTES;

/// A head's room's rows at least where its reach is not its room (`encode_place`).
const PLACE_ROOM_MIN: usize = 1 << 16;

/// Each head's columns' bounds (least, greatest), for the rows its rules can make: a constant's column its value, an
/// input relation's column its rows' (`InRel::col_range`), a column of a relation of the component that relation's
/// bounds, and the rows each head holds already (`Rel::ranges`); to a fixed point, as a recursive head's rows come from
/// its own. A value computed (a text made, a view's column) is unbounded: `[MIN, MAX]`. `[MAX, MIN]`: no rows.
pub(in crate::code_rule_dsl) fn head_bounds(rules: &[(&Plan, &JoinRule, usize)], store: &Store) -> HashMap<usize, Vec<[i64; 2]>> {
    const ALL: [i64; 2] = [i64::MIN, i64::MAX];
    let join = |a: [i64; 2], b: [i64; 2]| [a[0].min(b[0]), a[1].max(b[1])];
    let mut b: HashMap<usize, Vec<[i64; 2]>> = HashMap::new();
    for r in rules {
        b.entry(r.2).or_insert_with(|| store.rels[r.2].ranges.clone());
    }
    loop {
        let mut changed = false;
        for (plan, _, h) in rules {
            for c in 0..plan.head.len() {
                let v = match (plan.head[c], &plan.head_from[c]) {
                    (Some(k), _) => [k, k],
                    (None, Some(HeadFrom::In(r, col))) => r.col_range(*col).unwrap_or(ALL),
                    (None, Some(HeadFrom::Own(r, col))) => b.get(r).map_or_else(|| store.rels[*r].ranges.get(*col).copied().unwrap_or(ALL), |v| v.get(*col).copied().unwrap_or(ALL)),
                    (None, None) => ALL,
                };
                let cur = b.get_mut(h).expect("each head's bounds");
                let n = join(cur[c], v);
                if n != cur[c] {
                    cur[c] = n;
                    changed = true;
                }
            }
        }
        if !changed {
            return b;
        }
    }
}

/// A head's rows' sort key (t58 increment A): its first order (`Rel::first_order`), packed most significant first:
/// the leading column's 16-bit tag where the order is tagged (`runs::tag`), then each column of the order as its
/// value less its least, in the bits its bounds need (none for a column of one value). Its rows sorted by it are in the
/// order the host notes them in, so the host takes them as they lie.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::code_rule_dsl) struct HeadKey {
    pub tagged: bool,
    /// (column, least, bits), most significant first.
    pub parts: Vec<(usize, i64, u32)>,
    pub bits: u32,
}

impl HeadKey {
    /// The key of `order`'s rows under `bounds` (a column's least and greatest), where it fits 64 bits.
    pub(in crate::code_rule_dsl) fn of(order: &[usize], tagged: bool, bounds: &[[i64; 2]]) -> Option<HeadKey> {
        let mut parts = Vec::with_capacity(order.len());
        let mut bits = if tagged { 16 } else { 0 };
        for &c in order {
            let [lo, hi] = *bounds.get(c)?;
            // No rows: one value's width.
            let span = if hi < lo { 0 } else { hi.wrapping_sub(lo) as u64 };
            let w = 64 - span.leading_zeros();
            parts.push((c, if hi < lo { 0 } else { lo }, w));
            bits += w;
        }
        (bits <= 64).then_some(HeadKey { tagged, parts, bits })
    }
}

/// A run's head rows' key (t58 increment A): its head's `HeadKey` under `bounds`, each part the column of its run's
/// distinct rows (`uniq_cols`) that holds it. A column this rule makes a constant holds one value of its rows: no
/// bits, so none where its bounds give it some (another rule's values); a constant leading column leaves its rows'
/// tags all one, so the key is not tagged.
fn run_key(plan: &Plan, uniq_cols: &[usize], rel: &super::Rel, bounds: &[[i64; 2]]) -> Option<mj::SortKey> {
    if rel.folds() || rel.k == 0 || rel.k > 16 || plan.head.len() != rel.k || plan.arena.iter().any(|&a| a) {
        return None;
    }
    let (order, tagged) = rel.first_order();
    let hk = HeadKey::of(order, tagged, bounds)?;
    let mut parts = Vec::with_capacity(hk.parts.len());
    for &(c, lo, w) in &hk.parts {
        match plan.head[c] {
            Some(_) if w == 0 => {}
            Some(_) => return None,
            None => parts.push((uniq_cols.iter().position(|&u| u == plan.head_slots[c])? as u32, w, lo)),
        }
    }
    let tagged = hk.tagged && plan.head[order[0]].is_none();
    Some(mj::SortKey { tagged, parts, bits: hk.bits })
}

/// `place`'s work encoded into the stratum's own program, after its joins (decision 0603: a stratum is one launch):
/// each head's union of its rules' distinct rows from their counts on the device (`append_rows`, a placed table),
/// made distinct, less the rows the head holds, placed into room for the rows the head can reach (`reach`, within
/// `PLACE_ROOM_BYTES`; else eight times the rows guessed, within it). A gate holds each head's rows within that room;
/// past it the program stops there, and the heads are placed by `place` once the runs are done. None where no head
/// can be placed so (the heads `place` leaves to the rows read back): `place` then.
fn encode_place(runs: &[Run<'static>], rules: &[(&Plan, &JoinRule, usize)], store: &Store, fresh: &[Fresh], rep: &mut Report, b: &mut mj::Batch) -> Result<Option<PrePlaced>, String> {
    fn e(e: impl std::fmt::Debug) -> String {
        format!("{e:?}")
    }
    let mut heads: Vec<usize> = rules[..runs.len()].iter().map(|r| r.2).collect();
    heads.sort_unstable();
    heads.dedup();
    let (reaches, _) = reach(&rules[..runs.len()], &heads, store);
    // Each head's bounds, for its rows' sort key (t58 increment A).
    let bounds = head_bounds(&rules[..runs.len()], store);
    let mut pre = PrePlaced { heads: Vec::new(), pending: Vec::new(), _hold: (Vec::new(), Vec::new()) };
    let mut made: Vec<(usize, mj::Table, Option<usize>, mj::Words, usize, bool)> = Vec::new();
    for h in heads {
        let rel = &store.rels[h];
        let k = rel.k;
        let from: Vec<usize> = (0..runs.len()).filter(|&x| rules[x].2 == h).collect();
        if from.iter().any(|&x| rules[x].0.arena.iter().any(|&a| a)) || rel.folds() || k == 0 || k > 16 || (rel.n > 0 && k > mj::MAX_KEYS) {
            continue;
        }
        let guess: usize = from.iter().map(|&x| runs[x].uniq.as_ref().map_or(0, |u| u.cap)).sum();
        // Its room: the rows it can reach, where they fit `PLACE_ROOM_BYTES`; else eight times the rows guessed (at least
        // `PLACE_ROOM_MIN`), within it, the gate holding the rows to it.
        let most = PLACE_ROOM_BYTES / (8 * k);
        let bound = match reaches.get(&h).copied() {
            Some(r) if r <= most => r,
            _ => guess.saturating_mul(8).max(PLACE_ROOM_MIN).min(most),
        }
        .max(1);
        pre.heads.push(h);
        if guess == 0 {
            continue;
        }
        // The union, in the head's columns, its rows the rules' counts on the device.
        let union = b.table(k, guess).map_err(e)?;
        let words = mj::Words::zeroed(3).map_err(e)?;
        let cols: Vec<Vec<mj::Col>> = from.iter().map(|&x| {
            let (plan, r) = (rules[x].0, &runs[x]);
            plan.head.iter().zip(&plan.head_slots).map(|(c, s)| match c {
                Some(v) => mj::Col::Const(*v),
                None => mj::Col::From(r.uniq_cols.iter().position(|u| u == s).unwrap_or(0)),
            }).collect()
        }).collect();
        let srcs: Vec<(&mj::Table, usize, &mj::Words, usize, &[mj::Col])> = from.iter().zip(&cols).map(|(&x, c)| {
            let r = &runs[x];
            let u = r.uniq.as_ref().expect("the head's distinct rows");
            (u, u.cap, &r.totals, r.totals.len - 1, c.as_slice())
        }).collect();
        b.append_rows(&srcs, &union, (&words, 0)).map_err(e)?;
        let all: Vec<usize> = (0..k).collect();
        // Its rows' key in its first order, where its bounds pack it in 64 bits (t58 increment A): made distinct in
        // that order, so the host takes them as they lie (the stage less its held rows, and the place, keep it).
        let (order, tagged) = rel.first_order();
        let key = bounds.get(&h).and_then(|bd| HeadKey::of(order, tagged, bd)).map(|hk| mj::SortKey {
            tagged: hk.tagged,
            parts: hk.parts.iter().map(|&(c, lo, w)| (c as u32, w, lo)).collect(),
            bits: hk.bits,
        });
        // Made distinct: one rule's rows are already, and in the first order where its run sorted them (`run_key`).
        let one_sorted = from.len() == 1 && runs[from[0]].key.is_some();
        let key = key.filter(|_| from.len() > 1);
        let (rows, slot) = if let Some(key) = &key {
            let distinct = b.table(k, guess).map_err(e)?;
            b.dedup_sorted(&union, &all, 0, 1, &words, &distinct, None, key).map_err(e)?;
            pre._hold.0.push(union);
            (distinct, Some(1))
        } else if from.len() > 1 {
            let distinct = b.table(k, guess).map_err(e)?;
            b.dedup(&union, &all, 0, 1, &words, &distinct).map_err(e)?;
            pre._hold.0.push(union);
            (distinct, Some(1))
        } else {
            (union, Some(0))
        };
        // Less the rows the head holds, read where they lie.
        let (rows, slot) = match rel.in_block() {
            Some(held) if rel.n > 0 => {
                let dev = mj::DevRel::in_block(&held, k).map_err(|e| format!("{e:?}: a relation in its block"))?;
                let ix = b.index(&dev, &all).map_err(|e| format!("{e:?}: an index of {} rows", dev.rows))?;
                rep.in_place += 1;
                if ix.built { rep.sorts += 1 } else { rep.sort_hits += 1 }
                let mut d = mj::Stage { mode: mj::ANTI, nkeys: k as u32, ..mj::Stage::default() };
                for c in 0..k {
                    d.key_rcol[c] = c as u32;
                    d.key_kind[c] = mj::SLOT;
                    d.key_slot[c] = c as u32;
                }
                let fresh_rows = b.table(k, guess).map_err(e)?;
                let counters = mj::Words::zeroed(1).map_err(e)?;
                b.stage(d, &rows, Some((&dev, Some(&ix))), None, slot.unwrap_or(0), 2, &words, &counters, None, &fresh_rows).map_err(e)?;
                pre._hold.0.push(rows);
                pre._hold.1.push((dev, ix));
                (fresh_rows, Some(2))
            }
            _ => (rows, slot),
        };
        made.push((h, rows, slot, words, bound, key.is_some() || one_sorted));
    }
    if pre.heads.is_empty() {
        return Ok(None);
    }
    // The gate: each head's rows within the room reserved for them, before any is written there.
    let checks: Vec<(mj::At<'_>, Option<mj::At<'_>>, usize)> = made.iter().map(|(_, _, slot, words, bound, _)| (mj::At::Word(words, slot.unwrap_or(0)), None, *bound)).collect();
    b.gate(&checks).map_err(e)?;
    drop(checks);
    for (h, rows, slot, words, bound, sorted) in made {
        let k = store.rels[h].k;
        let (room, dst, at) = head_room(&store.rels[h], &fresh[h], k, bound)?;
        let ident: Vec<mj::Col> = (0..k).map(mj::Col::From).collect();
        b.place_rows(mj::Rows::Table(&rows), bound, slot, &words, &ident, dst, at).map_err(|e| format!("{e:?}: a head's rows placed"))?;
        pre._hold.0.push(rows);
        pre.pending.push((h, room, slot.unwrap_or(0), words, sorted));
    }
    Ok(Some(pre))
}

/// Each head's rows from `runs` (finished), written in place on the device (decision 0325 step 4): the union of its
/// rules' distinct rows (each rule's head columns, constants among them), made distinct when more than one rule makes
/// them, less the rows the head holds (read where they lie, its dead rows skipped), into a relation of a block whose
/// chunks the host reserved for them all (a rule's distinct count is known here, between the passes). The store then
/// takes that relation as it lies (`Fresh::place`): no row is read back. One submission for the stratum's heads.
///
/// A head is left to the rows read back (not in the result) when a rule of it makes texts (arena values, symbols only
/// once the host names them), when it folds (a lattice), or when the rows it holds have more columns than a probe's
/// keys.
fn place(runs: &[Run<'static>], rules: &[(&Plan, &JoinRule, usize)], store: &Store, fresh: &[Fresh], rep: &mut Report) -> Result<PlacedHeads, String> {
    let mut placed = PlacedHeads::default();
    let mut heads: Vec<usize> = rules.iter().map(|r| r.2).collect();
    heads.sort_unstable();
    heads.dedup();
    let mut batch: Option<mj::Batch> = None;
    // Per head: its part, the slot its count is in (`None`: the union's, known), the words, and what the work reads.
    let mut pending: Vec<(usize, super::Placed, Option<usize>, usize, mj::Words)> = Vec::new();
    let mut hold: Vec<mj::Table> = Vec::new();
    let mut hold_rels: Vec<(mj::DevRel, mj::DevIndex)> = Vec::new();
    for h in heads {
        let rel = &store.rels[h];
        let k = rel.k;
        let from: Vec<(usize, usize)> = runs.iter().enumerate().filter(|&(x, _)| rules[x].2 == h).map(|(x, r)| (x, r.totals.get(r.totals.len - 1) as usize)).collect();
        if from.iter().any(|&(x, _)| rules[x].0.arena.iter().any(|&a| a)) || rel.folds() || k == 0 || k > 16 || (rel.n > 0 && k > mj::MAX_KEYS) {
            continue;
        }
        placed.heads.push(h);
        let cap: usize = from.iter().map(|f| f.1).sum();
        if cap == 0 {
            continue;
        }
        let b = match &mut batch {
            Some(b) => b,
            None => batch.insert(direct_batch(None)?),
        };
        // The union, in the head's columns.
        let union = mj::Table::new(k, cap).map_err(|e| format!("{e:?}: a union of {cap} rows"))?;
        let mut at = 0;
        for &(x, n) in &from {
            let (plan, r) = (rules[x].0, &runs[x]);
            let cols: Vec<mj::Col> = plan.head.iter().zip(&plan.head_slots).map(|(c, s)| match c {
                Some(v) => mj::Col::Const(*v),
                None => mj::Col::From(r.uniq_cols.iter().position(|u| u == s).unwrap_or(0)),
            }).collect();
            b.copy_rows(mj::Rows::Table(r.uniq.as_ref().expect("the head's distinct rows")), n, &cols, mj::Rows::Table(&union), at).map_err(|e| format!("{e:?}"))?;
            at += n;
        }
        let words = mj::Words::zeroed(3).map_err(|e| format!("{e:?}"))?;
        words.set(0, cap as u32);
        let all: Vec<usize> = (0..k).collect();
        // Made distinct: one rule's rows are already.
        let (rows, slot) = if from.len() > 1 {
            let distinct = mj::Table::new(k, cap).map_err(|e| format!("{e:?}"))?;
            b.dedup(&union, &all, 0, 1, &words, &distinct).map_err(|e| format!("{e:?}"))?;
            hold.push(union);
            (distinct, Some(1))
        } else {
            (union, None)
        };
        // Less the rows the head holds, read where they lie.
        let (rows, slot) = match rel.in_block() {
            Some(held) if rel.n > 0 => {
                let dev = mj::DevRel::in_block(&held, k).map_err(|e| format!("{e:?}: a relation in its block"))?;
                let ix = b.index(&dev, &all).map_err(|e| format!("{e:?}: an index of {} rows", dev.rows))?;
                rep.in_place += 1;
                if ix.built { rep.sorts += 1 } else { rep.sort_hits += 1 }
                let mut d = mj::Stage { mode: mj::ANTI, nkeys: k as u32, ..mj::Stage::default() };
                for c in 0..k {
                    d.key_rcol[c] = c as u32;
                    d.key_kind[c] = mj::SLOT;
                    d.key_slot[c] = c as u32;
                }
                let fresh_rows = mj::Table::new(k, cap).map_err(|e| format!("{e:?}"))?;
                let counters = mj::Words::zeroed(1).map_err(|e| format!("{e:?}"))?;
                // A stage reads its bindings' count from its in-slot: the union's is the cap (slot 0).
                b.stage(d, &rows, Some((&dev, Some(&ix))), None, slot.unwrap_or(0), 2, &words, &counters, None, &fresh_rows).map_err(|e| format!("{e:?}"))?;
                hold.push(rows);
                hold_rels.push((dev, ix));
                (fresh_rows, Some(2))
            }
            _ => (rows, slot),
        };
        let (room, dst, at) = head_room(rel, &fresh[h], k, cap)?;
        let ident: Vec<mj::Col> = (0..k).map(mj::Col::From).collect();
        b.place_rows(mj::Rows::Table(&rows), cap, slot, &words, &ident, dst, at).map_err(|e| format!("{e:?}: a head's rows placed"))?;
        hold.push(rows);
        pending.push((h, room, slot, cap, words));
    }
    if let Some(mut b) = batch {
        finish_timed(&mut b, rep)?;
    }
    drop((hold, hold_rels));
    for (h, room, slot, cap, words) in pending {
        let n = slot.map_or(cap, |s| words.get(s) as usize).min(cap);
        // SAFETY: the batch is finished, and `rows_place` wrote every column of rows `0..n` of the room (`n` within
        // `cap`, the rows `head_room` reserved).
        let part = unsafe { written(room, n) };
        rep.distinct += n as u64;
        rep.placed += n as u64;
        placed.parts.push((h, part, false));
    }
    Ok(placed)
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
    /// The growths its rounds saw (`counted`), by rule and stage: a round's next is guessed from them, in the
    /// component's own order, before the run's memo as filed (decision 0478 step 7).
    growth: HashMap<(u64, usize), f64>,
    /// Each relation's keys (`delta`: its last round's), every one a stage of the component's plans probes it by: its
    /// deltas are made with room for an index of each, so no probe copies them (decision 0353 step 6,
    /// `DevIndex::moved`). A total grows in a block with room for its indexes (`SetHead`).
    keys: HashMap<(usize, bool), Vec<Vec<usize>>>,
    /// Each live relation's rows (`delta`: its delta's) as the last program left them (`device_rounds`): a stage's
    /// guess of its rows out reads these, not the relation's bound.
    now: HashMap<(usize, bool), usize>,
}

impl Own {
    /// Each relation's keys (`Own::keys`) gathered from the component's plans.
    fn keyed<'a>(plans: impl Iterator<Item = &'a Plan>) -> HashMap<(usize, bool), Vec<Vec<usize>>> {
        let mut keys: HashMap<(usize, bool), Vec<Vec<usize>>> = HashMap::new();
        let mut add = |at: (usize, bool), cols: Vec<usize>| {
            let ks = keys.entry(at).or_default();
            if !cols.is_empty() && !ks.contains(&cols) {
                ks.push(cols);
            }
        };
        for plan in plans {
            for s in &plan.stages {
                if let Kind::Own { rel, delta, cols } = &s.kind {
                    add((*rel, *delta), cols.clone());
                }
            }
        }
        keys
    }

    /// How many indexes relation `rel`'s (`delta`: its last round's) has room for.
    fn room(&self, rel: usize, delta: bool) -> usize {
        self.keys.get(&(rel, delta)).map_or(1, |k| k.len().max(1))
    }

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
        // A sort an earlier round's batch made is found again where its relation keeps it (`mj::Batch::again`): that
        // batch may have given it back (another batch built the same one, or its builder was cut by an overflow), and
        // a given-back index reads no rows. The cache handed it out as it was before: the next round's probes then
        // derived nothing from it.
        if let Some(ix) = self.sorts.get(&key).cloned() {
            let ix = batch.again(&dev, &ix).map_err(|e| format!("{e:?}: an index of {} rows found again", dev.rows))?;
            if ix.built { rep.sorts += 1 } else { rep.sort_hits += 1 }
            self.sorts.insert(key, ix.clone());
            return Ok(Some((dev, Some(ix))));
        }
        let ix = Rc::new(batch.index(&dev, cols).map_err(|e| format!("{e:?}: an index of {} rows", dev.rows))?);
        if ix.built { rep.sorts += 1 } else { rep.sort_hits += 1 }
        rep.moved += u32::from(ix.copied());
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

/// A round's rules encoded into `batch`: each one's head rows made distinct on the device, with their count. A rule
/// over an own relation with no rows, or over an empty input (`reads_empty_input`), is not run (`None`).
fn round_runs<'p>(rules: &[(&'p Plan, &JoinRule, usize)], own: &mut Own, names: &Interner, rep: &mut Report, batch: &mut mj::Batch, sizes: &HashMap<(usize, usize), usize>) -> Result<Vec<Option<Run<'p>>>, String> {
    let mut runs: Vec<Option<Run<'p>>> = Vec::with_capacity(rules.len());
    for (x, (plan, j, _)) in rules.iter().enumerate() {
        if !own.ready(plan) || super::reads_empty_input(j) {
            runs.push(None);
            continue;
        }
        let mut r = Run::new(plan)?;
        // Its tables' rows as a program of rounds sized them (`device_rounds`), else guessed as it encodes.
        for (o, c) in r.caps.iter_mut().enumerate().skip(1) {
            if let Some(&n) = sizes.get(&(x, o)) {
                *c = n;
            }
        }
        runs.push(Some(r));
    }
    // The round's rules level-major, their stages at each depth launched together (decision 0601).
    encode_levels(&mut runs.iter_mut().flatten().map(|r| (r, 0)).collect::<Vec<_>>(), batch, rep, own, names)?;
    Ok(runs)
}

/// Head `h`'s rows of a round from every rule of `rules` that makes it (`runs`), end to end in one table on the device
/// (`append_rows`), their count in slot 0 of words of 3 (`[union, distinct, new]`), and the rows guessed: none when no
/// rule of it ran. The table is placed by its rows (decision 0603 step 5d); the head gate bounds them
/// (`encode_live_tail`).
fn union_of(runs: &[Option<Run<'_>>], rules: &[(&Plan, &JoinRule, usize)], h: usize, store: &Store, named: &HashMap<i64, i64>, batch: &mut mj::Batch) -> Result<Option<(mj::Table, mj::Words, usize)>, String> {
    let text_head = matches!(store.rels[h].device_lattice(), Some(Ok((3, ..))));
    let k = store.rels[h].k;
    let from: Vec<(usize, &Run<'_>)> = runs.iter().enumerate().filter_map(|(x, r)| r.as_ref().filter(|_| rules[x].2 == h).map(|r| (x, r))).collect();
    let cap: usize = from.iter().map(|(_, r)| r.uniq.as_ref().map_or(0, |u| u.cap)).sum();
    if cap == 0 {
        return Ok(None);
    }
    let union = batch.table(k, cap).map_err(|e| format!("{e:?}: a union of {cap} rows"))?;
    let words = mj::Words::zeroed(3).map_err(|e| format!("{e:?}"))?;
    let cols: Vec<Vec<mj::Col>> = from.iter().map(|(x, r)| {
        let plan = rules[*x].0;
        plan.head.iter().zip(&plan.head_slots).enumerate().map(|(col, (c, s))| match c {
            // A shortlex_min value named as a constant: its text in the arena (decision 0307).
            Some(v) if col == k - 1 && text_head => mj::Col::Const(named[v]),
            Some(v) => mj::Col::Const(*v),
            None => mj::Col::From(r.uniq_cols.iter().position(|u| u == s).unwrap_or(0)),
        }).collect()
    }).collect();
    let srcs: Vec<(&mj::Table, usize, &mj::Words, usize, &[mj::Col])> = from.iter().zip(&cols).map(|((_, r), c)| {
        let u = r.uniq.as_ref().expect("the head's rows");
        (u, u.cap, &r.totals, r.totals.len - 1, c.as_slice())
    }).collect();
    batch.append_rows(&srcs, &union, (&words, 0)).map_err(|e| format!("{e:?}"))?;
    Ok(Some((union, words, cap)))
}

/// The gate after a round's joins (`mj::Batch::gate`): every stage's table of every run, by its room.
fn round_gate(runs: &[Option<Run<'_>>], batch: &mut mj::Batch) -> Result<(), String> {
    // A placed table's room is its rows: not checked.
    let checks: Vec<(mj::At<'_>, Option<mj::At<'_>>, usize)> = runs
        .iter()
        .flatten()
        .flat_map(|r| r.io.iter().filter(|(i, o)| i != o && !r.tables[*o].as_ref().is_some_and(mj::Table::placed)).map(move |&(_, o)| (mj::At::Word(&r.totals, o), None, r.caps[o])))
        .collect();
    batch.gate(&checks).map_err(|e| format!("{e:?}"))
}

/// The `Done` of a batch's work into `rep`.
fn tally(rep: &mut Report, done: &mj::Done) {
    rep.submissions += done.submissions;
    rep.dispatches += done.dispatches;
    rep.relaunches += done.relaunches;
    rep.remaps += done.remaps;
    rep.room_remaps += done.room_remaps;
    rep.taken += done.taken;
    rep.programs += done.programs;
    if done.launch_items != 0 {
        rep.launch_items = if rep.launch_items == 0 { done.launch_items } else { rep.launch_items.min(done.launch_items) };
    }
    rep.gpu_seconds += done.gpu_seconds;
    rep.levels += done.levels;
    rep.screened += done.screened;
}

/// A recursive component's fixpoint on the device: its seed joins `seeds` once, then its delta joins `deltas` in
/// rounds, each round's new rows (made distinct, less the rows the relation has) its next delta, until a round makes
/// none. Each relation's rows stay on the device between rounds; the host reads one count per relation a round, and
/// the new rows once, at the end, into `fresh` as the CPU's waves would add them. `heads` are the component's
/// relations, which must be empty when it starts. Returns each rule's rows made, seeds then deltas.
pub(in crate::code_rule_dsl) fn fixpoint(seeds: &[(&Plan, &JoinRule, usize)], deltas: &[(&Plan, &JoinRule, usize)], heads: &[usize], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    fixpoint_(seeds, deltas, heads, store, names, fresh)
}

fn fixpoint_(seeds: &[(&Plan, &JoinRule, usize)], deltas: &[(&Plan, &JoinRule, usize)], heads: &[usize], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    on_lane("stratum rounds", seeds.len() + deltas.len(), || fixpoint_here(seeds, deltas, heads, store, names, fresh))
}

/// A lattice head of a device fixpoint (decision 0384 step 5): its fold; its relation, made at its first round, with
/// room in its block to grow there; its rows held (`had`); its key map, in its block.
struct LatHead { code: mj::LatCode, part: Option<Part>, had: usize, map: Option<super::keys::KeyMap> }

/// A lattice head's block's room past its first round's rows: address space, memory only as its rows and key map are
/// written. A head past it refuses its stratum.
const LAT_ROOM: usize = 256 << 20;

/// The room a recursive set head's relation has in its block to grow, round after round (`SetHead`): address space,
/// memory only as its rows and indexes are written.
const HEAD_ROOM: usize = LAT_ROOM;

/// A recursive component's set head on the device: its one relation, in a block of the run's life, its rows so far
/// claimed and each round's new rows written past them (never moved; its indexes forgotten as it grows, so a round's
/// probes build them of its rows then). The store takes it as it lies (`Fresh::place`).
///
/// Its map (`map`, decision 0567): its rows by key, as a lattice head's key map is laid out, in its block. Each program
/// places its rows in it first (`mj::Batch::set_map`), at twice the program's bound; a round's new rows are its union
/// made distinct less the rows the map holds, each placed in it as the row it becomes (`mj::Batch::dedup_less`), so a
/// round's work is its own rows', not the head's.
#[derive(Default)]
struct SetHead {
    part: Option<Part>,
    map: Option<super::keys::KeyMap>,
}

/// A key map where it lies, for the device to bind.
fn map_at(m: &super::keys::KeyMap) -> mj::LatMap<'_> {
    mj::LatMap { home: m.facts().expect("a map in a block"), at: m.words().offset(), places: m.places() }
}

fn fixpoint_here(seeds: &[(&Plan, &JoinRule, usize)], deltas: &[(&Plan, &JoinRule, usize)], heads: &[usize], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
    let _in = in_device();
    let _span = crate::code_rule_dsl::logic::profile::span(|| "device fixpoint".into());
    let c0 = thread_cpu();
    let mut rep = Report { rules: (seeds.len() + deltas.len()) as u32, stages: seeds.iter().chain(deltas).map(|r| r.0.stages.len() as u32).sum(), ..Report::default() };
    // A made text crosses rounds only as a `shortlex_min` relation's value (decision 0307): its fold compares texts,
    // so one text made in two rounds (two arena values) is one value to it. Anywhere else its rounds would join or
    // make distinct texts by their arena values.
    let text_head = |h: usize| matches!(store.rels[h].device_lattice(), Some(Ok((3, ..))));
    for &(plan, _, h) in seeds.iter().chain(deltas) {
        let value = store.rels[h].k - 1;
        let makes = plan.stages.iter().any(|s| makes_text(s.desc.mode)) || plan.arena.iter().any(|&a| a);
        if makes && !(text_head(h) && plan.arena.iter().enumerate().all(|(c, &a)| !a || c == value)) {
            return Err("a made text in a recursive component outside a shortlex_min value (its rounds would join texts not yet symbols)".into());
        }
        if text_head(h) && !plan.arena.get(value).copied().unwrap_or(false) && plan.head.get(value).copied().flatten().is_none() {
            return Err("a shortlex_min value its rule neither makes nor names (decision 0307)".into());
        }
    }
    // The component's text arena, every round's (decision 0307), and the texts its rules name as constants there.
    let shared: Option<Rc<mj::Arena>> = if heads.iter().any(|&h| text_head(h)) {
        Some(Rc::new(mj::Arena::new(arena_max()).map_err(|e| format!("{e:?}: the component's text arena"))?))
    } else {
        None
    };
    let mut named: HashMap<i64, i64> = HashMap::new();
    if let Some(a) = &shared {
        for &(plan, _, h) in seeds.iter().chain(deltas) {
            let value = store.rels[h].k - 1;
            if let (true, Some(Some(v))) = (text_head(h), plan.head.get(value).copied()) && !named.contains_key(&v) {
                let text = u32::try_from(v).ok().and_then(|atom| names.atoms().with_str(atom, |t| t.as_bytes().to_vec())).ok_or("a shortlex_min constant that is no symbol")?;
                named.insert(v, a.put(&text).ok_or("the component's text arena too small for its constants")?);
            }
        }
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
    let mut own = Own { keys: Own::keyed(seeds.iter().chain(deltas).map(|r| r.0)), ..Own::default() };
    // A lattice head folds each round by key on the device (decision 0124), into its one relation through its key map
    // in its block (decision 0384 step 5).
    let mut lattices: HashMap<usize, LatHead> = HashMap::new();
    // A set head's one relation, grown round after round in its block (`SetHead`).
    let mut sets: HashMap<usize, SetHead> = HashMap::new();
    for &h in heads {
        if let Some(lat) = store.rels[h].device_lattice() {
            let (code, top, after) = lat?;
            lattices.insert(h, LatHead { code: mj::LatCode { code, top, after }, part: None, had: 0, map: None });
        }
    }
    // A head that keeps levels (decision 0311): each row's round, its level. A set head's rows are appended a round
    // at a time, so its rounds are kept here by row; a lattice head's are a column of its total (`lat_merge`'s).
    let leveled = |h: usize| store.rels[h].keeps_levels();
    let mut set_levels: HashMap<usize, Vec<u32>> = HashMap::new();
    let mut made = vec![0u64; seeds.len() + deltas.len()];
    // Every round on the device, the seed round's first: a program of them (`device_rounds`, decision 0603), whose
    // rounds go on across its launches.
    let mut rounds = 0;
    {
        let mut lp = Looped { own: &mut own, rep: &mut rep, lattices: &mut lattices, sets: &mut sets, set_levels: &mut set_levels, made: &mut made, rounds: &mut rounds };
        device_rounds(seeds, deltas, heads, store, names, &named, shared.as_ref(), &mut lp)?;
    }
    // A set head's relation as it lies (decision 0325 step 4): every row new (its relation had none before the run),
    // grown in its block round after round, each row with its round where the head keeps levels. Nothing copied.
    drop(own);
    for &h in heads {
        let Some(SetHead { part: Some(part), .. }) = sets.remove(&h) else { continue };
        let n = part.len() as usize;
        if n == 0 {
            continue;
        }
        // Its last round's indexes are of every row, in its block: the store's readers find them (decision 0325).
        let rounds = set_levels.get(&h).filter(|_| leveled(h)).map(|r| (0..n).map(|x| r.get(x).copied().unwrap_or(0)).collect());
        fresh[h].place(super::Placed::Part(part), rounds);
        rep.distinct += n as u64;
        rep.placed += n as u64;
    }
    // A lattice head's relation as it lies (decision 0384 step 5): one live row a key, folded in place round after
    // round, taken by the store with its key map. Nothing read back.
    let d0 = thread_cpu();
    for &h in heads {
        let Some(LatHead { part: Some(part), had, map: Some(map), .. }) = lattices.remove(&h) else { continue };
        if had == 0 {
            continue;
        }
        let k = store.rels[h].k;
        // A shortlex_min value is an arena text (decision 0307): stored as its symbol, put where the value lies.
        if let Some(a) = shared.as_ref().filter(|_| text_head(h)) {
            // copy: not block data: the head's distinct texts, interned at once.
            let mut values: Vec<i64> = (0..had as u32).map(|x| part.get(x, k - 1)).filter(|&v| v & mj::ARENA_TEXT != 0 && v != -1).collect();
            values.sort_unstable();
            values.dedup();
            let strs = values.iter().map(|&v| {
                let b = a.text(v).ok_or("a shortlex_min value past the arena")?;
                std::str::from_utf8(b).map_err(|_| "an arena text that is not UTF-8".to_owned())
            }).collect::<Result<Vec<&str>, String>>()?;
            let ids = names.atoms().intern_all(&strs).map_err(|e| format!("the component's texts' symbols: {e}"))?;
            let symbols: HashMap<i64, i64> = values.into_iter().zip(ids).map(|(v, id)| (v, i64::from(id))).collect();
            for x in 0..had as u32 {
                // The top (no arena text) stays as it is.
                if let Some(&id) = symbols.get(&part.get(x, k - 1)) {
                    // SAFETY: the device's work completed; this thread alone holds the relation now.
                    unsafe { part.put(x, k - 1, id) };
                }
            }
        }
        // Each row's round where the head keeps levels: its key's slot's.
        let rounds = leveled(h).then(|| {
            let mut r = vec![0u32; had];
            for (row, _, round) in map.each() {
                r[row as usize] = round;
            }
            r
        });
        fresh[h].place_folded(super::Placed::Part(part), rounds, map);
        rep.distinct += had as u64;
        rep.placed += had as u64;
    }
    rep.decode_cpu += thread_cpu() - d0;
    rep.rounds = rounds as u32;
    rep.encode_cpu = (thread_cpu() - c0 - rep.wait_cpu - rep.decode_cpu - rep.upload_cpu).max(0.0);
    one_launch(&rep);
    LAST.with_borrow_mut(|l| *l = Some(rep));
    Ok(made)
}

/// The address space a recursive head's delta reserves in its block (`Part::for_writes_with_room`): room for its rows
/// and their indexes to grow in place, never moved.
const DELTA_ROOM: usize = LAT_ROOM;

/// Programs of rounds a component encodes at most: one more each time a gate stops a round past its tables' or its
/// heads' room, each with room past what that round needed.
const PROGRAMS_MAX: usize = 64;

/// What `fixpoint_here` keeps across a component's rounds, for its rounds on the device (`device_rounds`).
struct Looped<'a> {
    own: &'a mut Own,
    rep: &'a mut Report,
    lattices: &'a mut HashMap<usize, LatHead>,
    sets: &'a mut HashMap<usize, SetHead>,
    set_levels: &'a mut HashMap<usize, Vec<u32>>,
    made: &'a mut [u64],
    rounds: &'a mut usize,
}

/// A head in a program of rounds (decision 0603 step 3): its relation and its two deltas, each grown in place in its
/// block. A round reads one delta (its side) and writes the other. `bound`: its relation's rows at most within the
/// program (a gate stops a round that could pass it); `room`: what a round adds, as guessed (twice what a stopped
/// one saw); `slack`: each delta's room, its relation's room past its rows when the program starts, within its bound:
/// a round's new rows (a lattice head's changed ones too) are at most those, so only the bound stops a round.
/// `mapped`: its rows' room as reserved, within its bound: where a mapped page is real memory (`mj::rooms_mapped`), a
/// set head's rows, its deltas' and its live indexes' are mapped as its rounds reach them, its gate's room growing
/// them between launches (`Batch::gate_rooms`); else its bound. `len`: its rows when the program started.
struct LiveHead {
    total: Part,
    deltas: [Part; 2],
    bound: usize,
    room: usize,
    slack: usize,
    mapped: usize,
    len: usize,
    /// Its rows mapped as its program's gates' rooms grew them (`room_growth`): one for all its rounds' gates.
    mapped_now: Rc<std::cell::Cell<usize>>,
}

impl LiveHead {
    /// Its deltas' rows mapped: a round's new rows at most within its mapped room.
    fn slack_mapped(&self) -> usize {
        self.mapped - self.len
    }

    /// Its live indexes' buckets: its bound's keys' (at most `LIVE_BUCKETS`), at least its rows mapped's.
    fn buckets(&self) -> usize {
        self.bound.min(LIVE_BUCKETS).max(self.mapped).next_power_of_two().max(1)
    }
}

/// A live relation's index's buckets at most past its first rows mapped (`LiveHead::buckets`): its rows grow into them
/// (a reader checks each row's key), 4 bytes a bucket.
const LIVE_BUCKETS: usize = 1 << 16;

/// A relation's bytes at most whose reach (`reach`) is its program's bound outright (`device_rounds`): a block's
/// room is real memory on Metal once bound, so a reach past this is a bound grown by what the rounds make.
const REACH_BYTES: usize = 64 << 20;

/// The rows each head of a recursive component can reach at most (decision 0603's fewer reruns): the product of its
/// columns' values at most. A column's values are those its rules copy into it: of a slot, a positive atom's column
/// that binds or keys it (the one of fewest values), a `let` of a constant or another slot; a head's own column through
/// the component's rules (a set of the base columns and constants it reaches, so a column copied into itself adds
/// nothing). A base column's values are its relation's rows at most. A head's rows are also at most its rules'
/// bindings, where no rule reads the component's own relations: the product of its positive atoms' rows (a split's
/// rows more: none). None for a head bounded neither way.
///
/// And the bytes a round's tables can reach in the program's arena (`arena`, none where a slot is unbounded so): each
/// rule's stage tables, a stage's rows at most the product of the values of the slots bound by then, its slots' words
/// a row (decision 0603 5c: rows × slots × 8 bytes), and its head's: its union's rows (its bindings') and its
/// distinct and new rows (its head's reach at most). The arena is emptied at each round's end, so a round's claims are all it holds.
fn reach(rules: &[(&Plan, &JoinRule, usize)], heads: &[usize], store: &Store) -> (HashMap<usize, usize>, Option<usize>) {
    use std::collections::BTreeSet;
    // A base column (its relation, by address or store index, and its column) with its rows, or a constant.
    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    enum Of {
        Col(usize, usize, usize),
        Const(i64),
    }
    type Dom = Option<BTreeSet<Of>>;
    fn size(d: &BTreeSet<Of>) -> usize {
        d.iter().map(|o| if let Of::Col(_, _, n) = o { *n } else { 1 }).fold(0usize, usize::saturating_add)
    }
    let mut dom: HashMap<(usize, usize), Dom> = HashMap::new();
    for &h in heads {
        for c in 0..store.rels[h].k {
            dom.insert((h, c), Some(BTreeSet::new()));
        }
    }
    let src_dom = |dom: &HashMap<(usize, usize), Dom>, src: &Src, col: usize| -> Dom {
        match src {
            Src::In(rel, _) => Some([Of::Col(Arc::as_ptr(rel) as usize, col, rel.rows() + rel.ndead)].into()),
            Src::Delta(r) | Src::Full(r) | Src::FullMulti(r, _) => match dom.get(&(*r, col)) {
                Some(d) => d.clone(),
                None => Some([Of::Col(usize::MAX - *r, col, store.rels[*r].n)].into()),
            },
        }
    };
    fn slot_dom(j: &JoinRule, s: usize, dom: &HashMap<(usize, usize), Dom>, src_dom: &dyn Fn(&HashMap<(usize, usize), Dom>, &Src, usize) -> Dom, depth: usize) -> Dom {
        if depth > 8 {
            return None;
        }
        let mut best: Dom = None;
        let mut take = |d: Dom| {
            if let Some(d) = d
                && best.as_ref().is_none_or(|b| size(&d) < size(b))
            {
                best = Some(d);
            }
        };
        for step in &j.steps {
            match step {
                Step::Atom { src, neg: false, keys, binds, .. } => {
                    for &(col, slot) in binds {
                        if slot == s {
                            take(src_dom(dom, src, col));
                        }
                    }
                    for (col, k) in keys {
                        if matches!(k, Key::Slot(x) if *x == s) {
                            take(src_dom(dom, src, *col));
                        }
                    }
                }
                Step::Let { a, b } if *a == s => take(match b {
                    Key::Const(v) => Some([Of::Const(*v)].into()),
                    Key::Slot(x) => slot_dom(j, *x, dom, src_dom, depth + 1),
                }),
                _ => {}
            }
        }
        best
    }
    // To a fixpoint: a head's columns' sets only grow, within the base columns and constants its rules name.
    for _ in 0..4 * dom.len() + 4 {
        let mut changed = false;
        for &(_, j, h) in rules {
            for (c, t) in j.out.iter().enumerate() {
                let add: Dom = match t {
                    Term::Slot(x) => slot_dom(j, *x, &dom, &src_dom, 0),
                    Term::Const(v) => Some([Of::Const(*v)].into()),
                    Term::Cat(..) => None,
                };
                let Some(cur) = dom.get_mut(&(h, c)) else { continue };
                match (cur.as_mut(), add) {
                    (None, _) => {}
                    (Some(_), None) => {
                        *cur = None;
                        changed = true;
                    }
                    (Some(set), Some(add)) => {
                        let n = set.len();
                        set.extend(add);
                        changed |= set.len() != n;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    // A rule's bindings at most, where none of its steps reads the component's own relations (which grow): the
    // product of its positive atoms' rows, every other step making a binding at most of one (a split, more: none).
    let in_component = |src: &Src| matches!(src, Src::Delta(r) | Src::Full(r) | Src::FullMulti(r, _) if heads.contains(r));
    let bindings = |j: &JoinRule| -> Option<usize> {
        let mut rows = 1usize;
        for step in &j.steps {
            match step {
                Step::Atom { src, neg: false, .. } if in_component(src) => return None,
                Step::Atom { src, neg: false, .. } => {
                    let n = match src {
                        Src::In(rel, _) => rel.rows() + rel.ndead,
                        Src::Delta(r) | Src::Full(r) | Src::FullMulti(r, _) => store.rels[*r].n,
                    };
                    rows = rows.saturating_mul(n);
                }
                Step::Split { .. } => return None,
                _ => {}
            }
        }
        Some(rows)
    };
    let heads: HashMap<usize, usize> = heads
        .iter()
        .filter_map(|&h| {
            let cols = (|| {
                let mut rows = 1usize;
                for c in 0..store.rels[h].k {
                    rows = rows.saturating_mul(size(dom[&(h, c)].as_ref()?));
                }
                Some(rows)
            })();
            let made = rules.iter().filter(|r| r.2 == h).map(|r| bindings(r.1)).try_fold(0usize, |a, b| b.map(|b| a.saturating_add(b)));
            match (cols, made) {
                (Some(a), Some(b)) => Some((h, a.min(b))),
                (a, b) => a.or(b).map(|r| (h, r)),
            }
        })
        .collect();
    let reach_of = |h: usize| heads.get(&h).copied();
    let arena = (|| {
        let mut bytes = 0usize;
        for &(_, j, h) in rules {
            let mut bound: BTreeSet<usize> = BTreeSet::new();
            let mut rows = 1usize;
            for step in &j.steps {
                let mut new: Vec<usize> = Vec::new();
                match step {
                    Step::Atom { neg: false, binds, .. } => new.extend(binds.iter().map(|&(_, s)| s)),
                    Step::Let { a, .. } => new.push(*a),
                    _ => continue,
                }
                for s in new {
                    if bound.insert(s) {
                        rows = rows.saturating_mul(size(slot_dom(j, s, &dom, &src_dom, 0).as_ref()?));
                    }
                }
                bytes = bytes.saturating_add(rows.saturating_mul(8 * j.nslots.max(1)));
            }
            // Its union holds its bindings' head rows; its distinct and new rows, its head's reach at most.
            let k = 8 * j.out.len().max(1);
            let head = reach_of(h).map_or(rows, |r| r.min(rows));
            bytes = bytes.saturating_add(rows.saturating_mul(k)).saturating_add(head.saturating_mul(2 * k));
        }
        Some(bytes)
    })();
    (heads, arena)
}

/// A program's first arena's bytes at most that a round's reach (`reach`) maps outright: its pages are real memory on
/// Metal once mapped, so past this the arena is mapped from the host's guess and grows as its claims reach it.
const ARENA_REACH_BYTES: usize = 256 << 20;

/// What a program of rounds reads and writes besides its relations, held until its launches are done; each
/// template's heads' ends, read when a gate stopped a round.
#[derive(Default)]
struct Kept {
    tables: Vec<mj::Table>,
    rels: Vec<Rc<mj::DevRel>>,
    maps: Vec<super::keys::KeyMap>,
    ends: Vec<Vec<HeadEnd>>,
}

/// A head's round (`encode_live_tail`): the words of its rows' counts (`[union, distinct, new]`; a set head's distinct
/// rows are its new ones, `dedup_less`), a lattice head's counters (`[changed, new]`), and the delta it writes.
struct HeadEnd {
    head: usize,
    words: mj::Words,
    lat: Option<mj::Words>,
    delta: Rc<mj::DevRel>,
    /// Its union's room (`union_of`'s): past it, its gate stopped the round.
    cap: usize,
}

impl HeadEnd {
    /// Its count the head gate reads (`encode_live_tail`): a set head's new rows, a lattice head's rows folded.
    fn slot(&self) -> usize {
        if self.lat.is_some() { 0 } else { 2 }
    }
}

/// The rounds after a component's first, on the device (decision 0603 step 3): a program of two rounds' templates (a
/// round reads one side of each head's deltas and writes the other), each a round's joins over the component's
/// relations as the device claims their rows, a gate on their tables' and heads' room, the heads' new rows onto their
/// relations and into the deltas, and the round's end, which claims them and goes on to the other template. Its rounds
/// go on across its launches, each its workers' budgets' (decision 0603 point 5); with no new rows the program ends.
/// Its arena is address space mapped as its claims reach it (`mj::Batch::arena_reserved`): it never stops a round. A gate that stops a round: the tables grow to the counts seen, and a program is encoded again from
/// that round (decision 0540's rerun).
#[allow(clippy::too_many_arguments)]
fn device_rounds<'p>(seeds: &[(&'p Plan, &JoinRule, usize)], deltas: &[(&'p Plan, &JoinRule, usize)], heads: &[usize], store: &Store, names: &Interner, named: &HashMap<i64, i64>,
                     shared: Option<&Rc<mj::Arena>>, lp: &mut Looped<'_>) -> Result<(), String> {
    fn e(e: impl std::fmt::Debug) -> String {
        format!("{e:?}")
    }
    let leveled = |h: usize| store.rels[h].keeps_levels() && !lp.lattices.contains_key(&h);
    // The round counted on the device: the next round's number.
    let round = mj::Words::zeroed(1).map_err(e)?;
    round.set(0, (*lp.rounds + 1) as u32);
    let start = *lp.rounds + 1;
    let all: Vec<(&Plan, &JoinRule, usize)> = seeds.iter().chain(deltas).copied().collect();
    let (reaches, arena_reach) = reach(&all, heads, store);
    // A set head's new rows by round, where it keeps levels (decision 0311).
    let mut logs: HashMap<usize, mj::Words> = HashMap::new();
    for &h in heads.iter().filter(|&&h| leveled(h)) {
        logs.insert(h, mj::Words::zeroed(ROUNDS_MAX + 2).map_err(e)?);
    }
    // Each rule's counts summed over the rounds (its run's totals'), and its tables' rows (a program's every round's);
    // the seed round's apart, and its runs once it has run.
    let mut sums: Vec<Option<mj::Words>> = (0..deltas.len()).map(|_| None).collect();
    let mut sizes: HashMap<(usize, usize), usize> = HashMap::new();
    let mut ssums: Vec<Option<mj::Words>> = (0..seeds.len()).map(|_| None).collect();
    let mut ssizes: HashMap<(usize, usize), usize> = HashMap::new();
    let mut seed_runs: Option<Vec<Option<Run<'p>>>> = None;
    // Each head's relation (made where an earlier run of the component left none) and its deltas.
    let mut live: HashMap<usize, LiveHead> = HashMap::new();
    for &h in heads {
        let k = store.rels[h].k;
        let total = match lp.lattices.get(&h) {
            Some(lh) => lh.part.clone(),
            None => lp.sets.get(&h).and_then(|s| s.part.clone()),
        };
        let total = match total {
            Some(p) => p,
            None => {
                let p = Part::for_writes_with_room(k, 1, if lp.lattices.contains_key(&h) { LAT_ROOM } else { HEAD_ROOM }).map_err(|e| format!("a recursive head's relation: {e}"))?;
                match lp.lattices.get_mut(&h) {
                    Some(lh) => lh.part = Some(p.clone()),
                    None => lp.sets.entry(h).or_default().part = Some(p.clone()),
                }
                p
            }
        };
        let fresh = || Part::for_writes_with_room(k, 1, DELTA_ROOM).map_err(|e| format!("a recursive head's delta: {e}"));
        live.insert(h, LiveHead { total, deltas: [fresh()?, fresh()?], bound: 0, room: 1024, slack: 0, mapped: 0, len: 0, mapped_now: Rc::default() });
    }
    let mut side = 0;
    let mut programs = 0;
    let mut rerun: Option<rerail_accel::gpu::launch::Rerun> = None;
    // Its programs' arenas' bytes at least (decision 0603 step 5): what a round's tables can reach where it fits
    // `ARENA_REACH_BYTES`, so its claims map no more pages, and twice what a stopped one's claims wanted.
    let mut floor = arena_reach.filter(|&b| b <= ARENA_REACH_BYTES).unwrap_or(0);
    // Whether its heads' rows are mapped as its rounds reach them (`LiveHead::mapped`).
    let rooms = mj::rooms_mapped();
    loop {
        programs += 1;
        if programs > PROGRAMS_MAX {
            return Err(format!("a recursive component's rounds encoded past {PROGRAMS_MAX} times"));
        }
        // Each head's room for the program: the rows it can reach where they fit `REACH_BYTES`, else its rows now and
        // twice what a round could add, within its reach; its deltas' its room past its rows now.
        for (&h, l) in live.iter_mut() {
            let len = l.total.len() as usize;
            let grown = 2 * (len + l.room);
            l.bound = match reaches.get(&h) {
                Some(&r) if r.saturating_mul(8 * store.rels[h].k) <= REACH_BYTES => r,
                Some(&r) => grown.min(r),
                None => grown,
            }
            .max(len + 1);
            l.slack = l.bound - len;
            // Its rows mapped (`LiveHead::mapped`): where a mapped page is real memory, a set head's its rows now and
            // twice what a round could add (or a test's rows past them), within its bound; else its bound.
            l.len = len;
            l.mapped = match rooms {
                Some(first) if !lp.lattices.contains_key(&h) => (len + if first == 0 { len + 2 * l.room } else { first }).min(l.bound),
                _ => l.bound,
            };
            l.mapped_now = Rc::new(std::cell::Cell::new(l.mapped));
            if !l.total.reserve(l.mapped as u64) || !l.deltas.iter().all(|d| d.reserve(l.slack_mapped() as u64)) {
                return Err(format!("a recursive head of {} rows past its block's room", l.mapped));
            }
            // Its indexes are the last program's, of rows it read then: each round builds its own.
            for p in std::iter::once(&l.total).chain(&l.deltas) {
                // SAFETY: no work is in flight (the last program's launches are done); its sorts are dropped below.
                unsafe { p.forget_indexes() };
            }
            lp.own.now.insert((h, false), len);
            lp.own.now.insert((h, true), l.deltas[side].len() as usize);
        }
        lp.own.sorts.clear();
        let r0 = round.get(0) as usize;
        let mut batch = match shared {
            Some(a) => shared_batch(a)?,
            None => direct_batch(None)?,
        };
        batch.arena_floor(floor);
        batch.arena_reserved(false);
        let mut kept = Kept::default();
        // Encoded, not launched, on an error: dropped with its batch, which nothing ran. The seed round first, until it
        // has run.
        let seeding = seed_runs.is_none().then_some((seeds, &mut ssizes, &mut ssums[..]));
        let (seeded, templates) = encode_program(seeding, deltas, heads, store, names, named, shared, lp, &live, &mut sizes, &mut sums, &logs, side, &round, &mut batch, &mut kept)?;
        let (t0, w0) = (Instant::now(), thread_cpu());
        let done = batch.finish().map_err(e)?;
        lp.rep.wait_wall += t0.elapsed().as_secs_f64();
        lp.rep.wait_cpu += thread_cpu() - w0;
        tally(lp.rep, &done);
        if let Some(why) = seeded.iter().flat_map(|s| &s.0).chain(templates.iter().flatten()).flatten().find_map(Run::failed) {
            return Err(why);
        }
        // The seed round stopped (`r0`'s) or ran.
        let r = round.get(0) as usize;
        let had_seed = seeded.is_some();
        let seed_stopped = done.stopped && had_seed && r == r0;
        let mut seed_ends = None;
        if let Some((runs, ends)) = seeded {
            if seed_stopped {
                seed_ends = Some((runs, ends));
            } else {
                seed_runs = Some(runs);
            }
        }
        if done.past_rounds {
            return Err(format!("a recursive component past {ROUNDS_MAX} rounds"));
        }
        if done.ended {
            // The rules' counts, summed over the rounds, as one run's each (`counted`): the seeds' then the deltas'.
            let mut runs: Vec<Option<Run<'_>>> = templates.into_iter().next().expect("a program's templates");
            let seeds_ran = seed_runs.take().expect("the seed round ran");
            for (base, rules, runs, sums) in [(0, seeds, seeds_ran, &mut ssums), (seeds.len(), deltas, std::mem::take(&mut runs), &mut sums)] {
                for (x, mut r) in runs.into_iter().enumerate() {
                    let (Some(r), Some(sum)) = (r.as_mut(), sums[x].take()) else { continue };
                    let (plan, j, _) = rules[x];
                    r.totals = sum;
                    let last = r.io.last().map_or(0, |io| io.1);
                    lp.rep.rows += u64::from(r.totals.get(last));
                    lp.made[base + x] += u64::from(r.totals.get(r.totals.len - 1));
                    for (k, g) in counted(r, plan, j, lp.rep) {
                        lp.own.growth.insert((plan.key, k), g);
                    }
                }
            }
            break;
        }
        if !done.stopped {
            return Err("a program of rounds left undone".into());
        }
        // A gate stopped round `r`: the seed round (`r0`, where the program ran it), else the template `(r - r1) %
        // 2`, `r1` the program's first round of templates. Its tables past their room grow to the counts seen (twice:
        // the rounds after it make more), its heads' rounds to their new rows (the head gate's, which ran where the
        // tables' passed), and a program is encoded again from it, reading its delta; its relations' bounds are made
        // again from their rows.
        // Its arena past its room: the next program's is twice what its claims wanted (decision 0603 step 5).
        if done.arena_full {
            floor = floor.max(2 * done.arena_wanted).max(batch.floor());
        }
        let (runs, ends, sizes) = match &seed_ends {
            Some((runs, ends)) => (runs, ends, &mut ssizes),
            None => {
                let r1 = r0 + usize::from(had_seed);
                let t = (r - r1) % 2;
                side ^= t;
                (&templates[t], &kept.ends[t], &mut sizes)
            }
        };
        for end in ends {
            let l = live.get_mut(&end.head).expect("a head");
            l.room = l.room.max(2 * end.words.get(end.slot()) as usize);
        }
        for (x, run) in runs.iter().enumerate() {
            let Some(run) = run else { continue };
            if let Some((s, n)) = run.overflow() {
                sizes.insert((x, run.io[s].1), 2 * n);
                for later in s + 1..run.plan.stages.len() {
                    let (i, o) = run.io[later];
                    if i != o {
                        sizes.remove(&(x, o));
                    }
                }
            }
        }
        drop(kept);
        lp.rep.reruns += 1;
        rerun.get_or_insert_with(rerail_accel::gpu::launch::rerun);
    }
    drop(rerun);
    // The rounds' rows claimed where they lie: a set head's levels by round, a lattice head's rows and keys.
    let end = round.get(0) as usize;
    *lp.rounds = end - 1;
    for (h, log) in &logs {
        let lv = lp.set_levels.entry(*h).or_default();
        for r in start..end {
            let n = log.get(r) as usize;
            lv.resize(lv.len() + n, r as u32);
        }
    }
    for (&h, lh) in lp.lattices.iter_mut() {
        lh.had = live[&h].total.len() as usize;
        lh.map.as_mut().expect("its key map").held(lh.had);
    }
    lp.own.total.clear();
    lp.own.delta.clear();
    lp.own.sorts.clear();
    Ok(())
}

/// A program of rounds (`device_rounds`) encoded into `batch`: a lattice head's key map grown to its bound; the seed
/// round where the seeds have not run (`seeding`: their rules, sizes and sums), writing the deltas of `side`; then two
/// rounds' templates, the first reading the deltas of `side`, each with its end. The seed round's runs and ends, and
/// each template's runs.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn encode_program<'p>(seeding: Option<(&[(&'p Plan, &JoinRule, usize)], &mut HashMap<(usize, usize), usize>, &mut [Option<mj::Words>])>, deltas: &[(&'p Plan, &JoinRule, usize)], heads: &[usize],
                      store: &Store, names: &Interner, named: &HashMap<i64, i64>, shared: Option<&Rc<mj::Arena>>, lp: &mut Looped<'_>, live: &HashMap<usize, LiveHead>,
                      sizes: &mut HashMap<(usize, usize), usize>, sums: &mut [Option<mj::Words>], logs: &HashMap<usize, mj::Words>, side: usize, round: &mj::Words, batch: &mut mj::Batch,
                      kept: &mut Kept) -> Result<(Option<(Vec<Option<Run<'p>>>, Vec<HeadEnd>)>, Vec<Vec<Option<Run<'p>>>>), String> {
    fn e(e: impl std::fmt::Debug) -> String {
        format!("{e:?}")
    }
    lp.own.total.clear();
    for (&h, l) in live {
        lp.own.total.insert(h, live_rel(&l.total, l.bound, store.rels[h].k, (l.mapped < l.bound).then(|| (l.mapped, l.buckets())))?);
    }
    // A lattice head's key map with room for its bound's keys at half full: grown once, before the rounds fold into it.
    for (&h, lh) in lp.lattices.iter_mut() {
        let need = (2 * live[&h].bound).next_power_of_two().max(16);
        if lh.map.as_ref().is_some_and(|m| m.places() >= need) {
            continue;
        }
        let part = &live[&h].total;
        // SAFETY: `lat_hclear_one` writes every slot before any read.
        let words = unsafe { BlockVec::unwritten_in(Some(&part.home()), need * super::keys::SLOT) };
        let map = super::keys::KeyMap::of(words, lh.had);
        // Its keys so far placed again in it (none where its first round made none).
        batch.lattice_grow(&lp.own.total[&h], store.rels[h].k - 1, map_at(&map), lh.map.as_ref().map(map_at)).map_err(e)?;
        kept.maps.extend(lh.map.replace(map));
    }
    // A set head's map with room for its bound's rows at half full, its rows placed in it again: a round its gate
    // stopped in the last program may have placed rows it never wrote.
    for (&h, l) in live {
        if lp.lattices.contains_key(&h) {
            continue;
        }
        let need = (2 * l.bound).next_power_of_two().max(16);
        let set = lp.sets.entry(h).or_default();
        if set.map.as_ref().is_none_or(|m| m.places() < need) {
            // SAFETY: `lat_hclear_one` (`set_map`'s first phase) writes every slot before any read.
            let words = unsafe { BlockVec::unwritten_in(Some(&l.total.home()), need * super::keys::SLOT) };
            kept.maps.extend(set.map.replace(super::keys::KeyMap::of(words, 0)));
        }
        batch.set_map(&lp.own.total[&h], l.len, map_at(set.map.as_ref().expect("its map"))).map_err(e)?;
    }
    // The seed round: its rules read no delta, and its rows are the first round's delta, which template 0 reads.
    let seeded = match seeding {
        Some((seeds, ssizes, ssums)) => Some(encode_round(seeds, None, side, None, heads, store, names, named, shared, lp, live, ssizes, ssums, logs, round, batch, kept)?),
        None => None,
    };
    let first = batch.next_phase().map_err(e)?;
    let mut templates = Vec::new();
    for t in 0..2 {
        let (runs, ends) = encode_round(deltas, Some(side ^ t), side ^ t ^ 1, (t == 1).then_some(first), heads, store, names, named, shared, lp, live, sizes, sums, logs, round, batch, kept)?;
        kept.ends.push(ends);
        templates.push(runs);
    }
    kept.rels.extend(lp.own.total.values().cloned());
    Ok((seeded, templates))
}

/// A recursive relation in its block as the device claims its rows, at most `rows`.
/// Head `l`'s room's growth for one of its rounds' gates (`mj::RoomGrow`), its round's new rows read by `new`: its rows
/// mapped past its rows and those, twice what they need (at least twice its room), within its bound; its deltas' with
/// them. Its rounds' gates share what it maps (`LiveHead::mapped_now`): a gate whose cap is behind it is given it.
fn room_growth(l: &LiveHead, new: impl Fn() -> u32 + 'static) -> mj::RoomGrow {
    let (total, deltas, bound, len0, now) = (l.total.clone(), l.deltas.clone(), l.bound, l.len, l.mapped_now.clone());
    let cap = std::cell::Cell::new(l.mapped);
    Box::new(move || {
        let need = total.len() as usize + new() as usize;
        // Past its room, within its bound (past it, the bound's check stops the round): mapped further.
        let mut rels = Vec::new();
        if need > now.get() && need <= bound {
            let to = (2 * need).max(2 * now.get()).min(bound);
            if !total.reserve(to as u64) || !deltas.iter().all(|d| d.reserve((to - len0) as u64)) {
                return Err(format!("a recursive head of {to} rows past its block's room"));
            }
            now.set(to);
            rels = std::iter::once((&total, to)).chain(deltas.iter().map(|d| (d, to - len0))).map(|(p, rows)| (p.facts().clone(), p.rel_at() as u64, rows as u32)).collect();
        }
        if now.get() <= cap.get() {
            return Ok(None);
        }
        cap.set(now.get());
        Ok(Some(mj::Grown { cap: now.get(), rels }))
    })
}

fn live_rel(p: &Part, rows: usize, k: usize, mapped: Option<(usize, usize)>) -> Result<Rc<mj::DevRel>, String> {
    let d = mj::DevRel::in_block(&p.in_block(rows as u32), k).map_err(|e| format!("{e:?}: a recursive relation in its block"))?.live();
    // Mapped as its rounds reach it: its indexes' room for its rows mapped (`LiveHead::mapped`).
    Ok(Rc::new(match mapped {
        Some((m, buckets)) => d.mapped(m, buckets),
        None => d,
    }))
}

/// One round of a program of rounds (`encode_program`): `rules`' joins over the component's relations as the device
/// claims their rows, reading the deltas of side `reads` (none: the seed round's rules read none); the gate on its
/// tables' room; its heads' new rows into the deltas of side `writes` (`encode_live_tail`); and its end: its heads'
/// new rows claimed onto their relations, its deltas' lengths, its words emptied for the next round, its runs' counts
/// summed (`sums`), on to phase `next` (none: the phase after it). Its runs and its heads' ends.
#[allow(clippy::too_many_arguments)]
fn encode_round<'p>(rules: &[(&'p Plan, &JoinRule, usize)], reads: Option<usize>, writes: usize, next: Option<u32>, heads: &[usize], store: &Store, names: &Interner, named: &HashMap<i64, i64>,
                    shared: Option<&Rc<mj::Arena>>, lp: &mut Looped<'_>, live: &HashMap<usize, LiveHead>, sizes: &mut HashMap<(usize, usize), usize>, sums: &mut [Option<mj::Words>],
                    logs: &HashMap<usize, mj::Words>, round: &mj::Words, batch: &mut mj::Batch, kept: &mut Kept) -> Result<(Vec<Option<Run<'p>>>, Vec<HeadEnd>), String> {
    fn e(e: impl std::fmt::Debug) -> String {
        format!("{e:?}")
    }
    lp.own.delta.clear();
    lp.own.sorts.clear();
    if let Some(reads) = reads {
        for (&h, l) in live {
            lp.own.delta.insert(h, live_rel(&l.deltas[reads], l.slack, store.rels[h].k, (l.mapped < l.bound).then(|| (l.slack_mapped(), l.buckets())))?);
        }
    }
    let runs = round_runs(rules, lp.own, names, lp.rep, batch, sizes)?;
    // The other template's tables as this one's: every round of the program alike.
    for (x, r) in runs.iter().enumerate() {
        for (o, &c) in r.iter().flat_map(|r| r.caps.iter().enumerate().skip(1)) {
            if c > 0 {
                sizes.insert((x, o), c);
            }
        }
    }
    // The gate: every stage's table by its room.
    round_gate(&runs, batch)?;
    let ends = encode_live_tail(&runs, rules, heads, store, lp, live, writes, round, named, shared, batch, kept)?;
    for (x, r) in runs.iter().enumerate() {
        if let (Some(r), None) = (r, &sums[x]) {
            sums[x] = Some(mj::Words::zeroed(r.totals.len).map_err(e)?);
        }
    }
    let mut ops: Vec<mj::RoundOp<'_>> = Vec::new();
    for end in &ends {
        let total = mj::At::Len(&lp.own.total[&end.head]);
        let (changed, added) = match &end.lat {
            Some(c) => (mj::At::Word(c, 0), mj::At::Word(c, 1)),
            None => (mj::At::Word(&end.words, 2), mj::At::Word(&end.words, 2)),
        };
        ops.push(mj::RoundOp::Add(total, added));
        ops.push(mj::RoundOp::Set(mj::At::Len(&end.delta), changed));
        ops.push(mj::RoundOp::Any(changed));
        if let Some(log) = logs.get(&end.head) {
            ops.push(mj::RoundOp::Log(log, added));
        }
        ops.push(mj::RoundOp::Zero(&end.words, 0, 3));
        if let Some(c) = end.lat.as_ref() {
            ops.push(mj::RoundOp::Zero(c, 0, c.len));
        }
    }
    for (x, r) in runs.iter().enumerate() {
        let Some(r) = r else { continue };
        ops.push(mj::RoundOp::Sum(sums[x].as_ref().expect("its sums"), &r.totals));
        ops.push(mj::RoundOp::Zero(&r.totals, 1, r.totals.len - 1));
    }
    batch.round_end(&ops, round, ROUNDS_MAX as u32, next).map_err(e)?;
    drop(ops);
    Ok((runs, ends))
}

/// A round's heads in a program of rounds (`encode_program`), after its joins: each head's rows from every rule that
/// makes it (`union_of`), a set head's made distinct less the rows its map holds (`SetHead::map`, its relation's as
/// the device claimed them); a gate on each head's room (its delta's for a round's rows, its relation's bound for them past its
/// rows); then a set head's new rows past its rows (`append_live`) and into its delta of side `writes`, a lattice
/// head's folded into its relation through its key map, its changed rows into that delta. Their counts the device's,
/// claimed by the round's end.
#[allow(clippy::too_many_arguments)]
fn encode_live_tail(runs: &[Option<Run<'_>>], rules: &[(&Plan, &JoinRule, usize)], heads: &[usize], store: &Store, lp: &mut Looped<'_>, live: &HashMap<usize, LiveHead>, writes: usize,
                    round: &mj::Words, named: &HashMap<i64, i64>, shared: Option<&Rc<mj::Arena>>, batch: &mut mj::Batch, kept: &mut Kept) -> Result<Vec<HeadEnd>, String> {
    fn e(e: impl std::fmt::Debug) -> String {
        format!("{e:?}")
    }
    // A head's rows of the round, before its gate: its union (and a set head's new rows).
    struct Made {
        end: HeadEnd,
        union: mj::Table,
        cap: usize,
        fresh: Option<mj::Table>,
    }
    // Head `h`'s delta of side `writes`, live: its room the head gate's.
    let into = |h: usize| -> Result<mj::DevRel, String> {
        let l = &live[&h];
        Ok(mj::DevRel::in_block(&l.deltas[writes].in_block(l.slack as u32), store.rels[h].k).map_err(|e| format!("{e:?}: a delta in its block"))?.live())
    };
    let mut made = Vec::new();
    for &h in heads {
        let k = store.rels[h].k;
        let Some((union, words, cap)) = union_of(runs, rules, h, store, named, batch)? else { continue };
        let delta = Rc::new(into(h)?);
        if lp.lattices.contains_key(&h) {
            made.push(Made { end: HeadEnd { head: h, words, lat: Some(mj::Words::zeroed(2).map_err(e)?), delta, cap }, union, cap, fresh: None });
            continue;
        }
        // Made distinct, less the rows its relation holds (its map's, `SetHead::map`): its new rows, their count the
        // words' third, which the head gate reads.
        let all: Vec<usize> = (0..k).collect();
        let fresh = batch.table(k, cap).map_err(e)?;
        let map = lp.sets.get(&h).and_then(|s| s.map.as_ref()).ok_or("a set head's map")?;
        batch.dedup_less(&union, &all, 0, 2, &words, &fresh, Some((&lp.own.total[&h], map_at(map)))).map_err(e)?;
        made.push(Made { end: HeadEnd { head: h, words, lat: None, delta, cap }, union, cap, fresh: Some(fresh) });
    }
    // The head gate: a round's rows past its relation's rows within its bound (a lattice head's union's rows: its new
    // keys and its changed rows at most). Its delta's room is its relation's past its rows when the program started
    // (`LiveHead::slack`), so within the bound the round's rows fit it. Where its rows are mapped as reached
    // (`LiveHead::mapped`), a second check of its room: past it the program parks, and its growth reserves its rows,
    // its deltas' and its live indexes' further (twice what it needs, within its bound) and the program goes on.
    let mut checks: Vec<(mj::At<'_>, Option<mj::At<'_>>, usize)> = Vec::new();
    let mut rooms_grow: Vec<(usize, mj::RoomGrow)> = Vec::new();
    for m in &made {
        let (l, n) = (&live[&m.end.head], mj::At::Word(&m.end.words, m.end.slot()));
        let total = &lp.own.total[&m.end.head];
        checks.push((mj::At::Len(total), Some(n), l.bound));
        if l.mapped < l.bound {
            rooms_grow.push((checks.len(), room_growth(l, m.end.words.reader(m.end.slot()))));
            checks.push((mj::At::Len(total), Some(mj::At::Word(&m.end.words, m.end.slot())), l.mapped));
        }
    }
    batch.gate_rooms(&checks, rooms_grow).map_err(e)?;
    drop(checks);
    let mut ends = Vec::new();
    // The folds together, then their rows' copies: a fold may not run twice, and the claims of the arena are all
    // before them (theirs at their level's start; after them, copies only).
    let mut copies = Vec::new();
    for Made { end, union, cap, fresh } in made {
        let h = end.head;
        let k = store.rels[h].k;
        let ident: Vec<mj::Col> = (0..k).map(mj::Col::From).collect();
        let total = lp.own.total[&h].clone();
        match (fresh, &end.lat) {
            (None, Some(counters)) => {
                let lh = &lp.lattices[&h];
                let changed = batch.table(k, cap).map_err(e)?;
                let lv = store.rels[h].keeps_levels().then_some(mj::LatLevel::Word(round));
                batch.lattice_fold(&union, cap, &total, 0, k - 1, lh.code, map_at(lh.map.as_ref().expect("its key map")), mj::MapFirst::Ready, counters, &changed, shared.map(|a| &**a), lv, Some((&end.words, 0))).map_err(e)?;
                copies.push((changed, cap, ends.len(), ident));
            }
            (Some(fresh), _) => {
                batch.append_live(mj::Rows::Table(&fresh), cap, (2, &end.words), &ident, &total).map_err(e)?;
                batch.place_rows(mj::Rows::Table(&fresh), cap, Some(2), &end.words, &ident, into(h)?, 0).map_err(e)?;
                kept.tables.push(fresh);
            }
            (None, None) => unreachable!("a set head's new rows"),
        }
        kept.tables.push(union);
        kept.rels.push(total);
        ends.push(end);
    }
    for (changed, cap, at, ident) in copies {
        let end: &HeadEnd = &ends[at];
        batch.place_rows(mj::Rows::Table(&changed), cap, Some(0), end.lat.as_ref().expect("a lattice head's counters"), &ident, into(end.head)?, 0).map_err(e)?;
        kept.tables.push(changed);
    }
    Ok(ends)
}

/// Each step's observed counts after `r` ran, as the CPU's walk makes them, into `j`'s (the replanner's fanouts),
/// and the growth seen, for the next chunk's guesses.
/// The run's counts into its rule's, and the growth each stage saw, for the next runs' guesses: (stage, rows out per
/// row of its input's space), filed into the joins' memo too.
fn counted(r: &Run<'_>, plan: &Plan, j: &JoinRule, rep: &mut Report) -> Vec<(usize, f64)> {
    let mut grew = Vec::new();
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
            // One witness per binding: every step of the rest passes the bindings that have one.
            (Kind::Witness { .. }, _) => vec![n_out; s.span],
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
        // The growth seen, for the next runs' guesses.
        if i != o && r.caps[i] > 0 {
            let g = n_out as f64 / r.caps[i] as f64;
            crate::code_rule_dsl::logic::plan::joins::note_growth(plan.key, k, g);
            grew.push((k, g));
        }
    }
    grew
}

#[cfg(test)]
pub mod tests;
#[cfg(test)]
mod parity;
