//! Where a stratum runs, CPU or GPU, from its shape and the planner's estimates (decision 0095 §8).
//!
//! Before a stratum opens, its seeds' and probes' plans give an estimate of its work: the planner's own unit, the
//! rows a join step reads. Each backend has a price for that work:
//!
//! - **CPU:** a floor per stratum (open, index building, close) and a cost per unit of work. Both were measured on
//!   the Python pack, cold, on 2026-11-09: 5,476 strata, 177.9 M units of work and 20.6 s of joins. Strata above
//!   10^5 units cost 92–109 ns a unit; the 3,372 strata under 10^3 units cost 0.13 ms each.
//! - **GPU:** a submission's fixed cost (submit, schedule, wake: decision 0039's phases) and a cost per unit, both
//!   **assumed**; and each **join step** priced on each form it can take (`price_join`): the CPU's index lookups, the
//!   device's sort and probe (any equi-join; a relation sorted once a stratum for every probe of it by the same
//!   keys), the device's tile (a range inside a document). The device's join prices are **measured** by the
//!   relational lab (0095); the step's estimates are the planner's (its recorded plan, kept when profiling or when
//!   the run may use the device).
//!
//! The stratum's **shape** is the relational forms its plans use: joins, negation, aggregates, comparisons,
//! computed terms, text tests, recursion, tree axes. The device runs a stratum only when it has every form
//! (`DEVICE_FORMS`), and only under the Metal binding. Otherwise the stratum runs on the CPU, and the profile says
//! where the GPU would have been cheaper and which forms it lacks: the work the device must take on next, by its cost.

use super::{Mode, Step};
use crate::code_rule_dsl::logic::front::{AggOp, Literal, Term};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

/// A relational form a stratum's plans use: what a backend must be able to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::code_rule_dsl) enum Form {
    /// A positive atom read by a join step.
    Join,
    /// A negated atom (an anti-join, a bitmap test).
    Negation,
    /// `count`, `min`, `max`, `sum`.
    Aggregate,
    /// A string join (`strjoin`): an ordered group.
    StringJoin,
    /// A comparison of plain terms.
    Comparison,
    /// A computed term (`@doc(n)`, arithmetic, `strlen`, a string join's call).
    Computed,
    /// A text test's relation (`strtest_<k>`, decision 0037).
    TextTest,
    /// A recursive stratum: delta probes, rounds to a fixpoint.
    Recursion,
    /// A tree axis read as a range, a climb or a descent (`within`, ancestors).
    TreeAxis,
    /// `nearest`: the nearest ancestor of a kind set.
    Nearest,
    /// An `ast` matcher, run by a transducer program.
    Matcher,
}

impl Form {
    pub(in crate::code_rule_dsl) fn name(self) -> &'static str {
        match self {
            Form::Join => "join",
            Form::Negation => "negation",
            Form::Aggregate => "aggregate",
            Form::StringJoin => "string join",
            Form::Comparison => "comparison",
            Form::Computed => "computed term",
            Form::TextTest => "text test",
            Form::Recursion => "recursion",
            Form::TreeAxis => "tree axis",
            Form::Nearest => "nearest",
            Form::Matcher => "ast matcher",
        }
    }
}

/// The forms the Metal backend runs (`setfix::device`, `metal_join`): joins of atoms by their keys (or whole),
/// negated atoms and atoms that bind nothing (anti- and semi-joins by the same keys), and comparisons, `in` and
/// document tests as row programs, recursion (a recursive stratum's rounds to its fixpoint on the device,
/// `device::fixpoint`; one that keeps levels is refused at open, named), and aggregates (count, sum, min, max folded
/// per binding over the relation's sort by the keys), and text tests with constant patterns of a slot an input
/// relation's column binds (verdicts per distinct symbol by the test's automaton, `text::table`; any other text test
/// is refused at open, named). A stratum that needs any other form stays on the CPU, named in the profile.
const DEVICE_FORMS: &[Form] = &[Form::Join, Form::Negation, Form::Comparison, Form::Recursion, Form::Aggregate, Form::TextTest];

/// A join step as the planner estimates it: the bindings it reads, the rows of the relation it reads, the rows it
/// makes, its key columns, and whether it reads a range inside a document (`Mode::Range`).
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct JoinEst {
    pub(in crate::code_rule_dsl) rel: String,
    pub(in crate::code_rule_dsl) input: f64,
    pub(in crate::code_rule_dsl) relation: f64,
    pub(in crate::code_rule_dsl) output: f64,
    pub(in crate::code_rule_dsl) keys: usize,
    pub(in crate::code_rule_dsl) range: bool,
}

/// How a join step runs: the CPU engine's index lookups, or on the device a sort and probe (any equi-join) or a
/// brute-force tile within each document (a range inside a document, where a key cannot find the pairs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::code_rule_dsl) enum JoinForm {
    Index,
    Probe,
    Tile,
}

impl JoinForm {
    pub(in crate::code_rule_dsl) fn name(self) -> &'static str {
        match self {
            JoinForm::Index => "CPU index",
            JoinForm::Probe => "GPU sort and probe",
            JoinForm::Tile => "GPU tile",
        }
    }
}

/// A join step's prices on each form (seconds; `None` where the form cannot run it), and the cheapest.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct JoinChoice {
    pub(in crate::code_rule_dsl) rel: String,
    pub(in crate::code_rule_dsl) form: JoinForm,
    pub(in crate::code_rule_dsl) index_s: f64,
    pub(in crate::code_rule_dsl) probe_s: Option<f64>,
    pub(in crate::code_rule_dsl) tile_s: Option<f64>,
}

/// The device's join prices, in seconds of host CPU and GPU together, measured on 2026-11-09 (an M-series GPU, the
/// machine loaded): the relational lab's replay of the Python pack's 144 captured strata (232 equi-joins, 5.6 M
/// pairs; 110 of them also as tiles) and `metal_sort`'s `host_costs` at 1,000 to 200,000 rows.
/// - A probe: its submission (about 55 µs of host and 12 µs of GPU), per left row the GPU's search, per key value
///   the host's span and pack, per pair the GPU's count and scatter and the host's read.
/// - A sort, once a stratum per relation and key: its submission (about 100 µs of host, 25 µs of GPU), per row the
///   GPU's radix passes and runs.
/// - A tile: its two submissions, per key value the host's sort by chunk and pack, per pair compared the GPU's test,
///   and the pairs compared per pair found (387 M for 5.6 M in the lab).
const UPLOAD_PER_VALUE: f64 = 0.3e-9;
const PROBE_FIXED: f64 = 70e-6;
const PROBE_PER_ROW: f64 = 1.2e-9;
const PROBE_PER_PAIR: f64 = 3e-9;
const SORT_FIXED: f64 = 125e-6;
const SORT_PER_ROW: f64 = 1.5e-9;
const TILE_FIXED: f64 = 150e-6;
const TILE_HOST_PER_VALUE: f64 = 5e-9;
const TILE_PER_COMPARE: f64 = 0.4e-9;
const TILE_COMPARES_PER_PAIR: f64 = 70.0;

/// The device's stratum, as `setfix::device` runs it (every seed join in one submission, its relations kept on the
/// device across the chunk's strata), in seconds of the host's CPU: a submission (the batch, its commit, the wait
/// and the completion's wake), a rule's tables and words, a stage's encoding (count, scan, scatter, about four
/// dispatches at 1.5 µs and their buffers), a sort's encoding (a hash and the radix passes, about 25 dispatches), and
/// a head row read back into the store. Measured on the Python pack's placed strata (`== Placement`'s "Strata run on
/// the GPU": host CPU against rules, stages and rows).
const DEVICE_SUBMISSION: f64 = 100e-6;
const DEVICE_RULE: f64 = 6e-6;
const DEVICE_STAGE: f64 = 8e-6;
const DEVICE_SORT: f64 = 45e-6;
const DEVICE_ROW_BACK: f64 = 40e-9;

/// CPU: seconds per unit of planned work, and per stratum (measured; see the module's comment).
const CPU_PER_WORK: f64 = 100e-9;
const CPU_PER_STRATUM: f64 = 0.13e-3;
/// GPU: seconds per unit of planned work, and per submission (assumed; see the module's comment).
const GPU_PER_WORK: f64 = 2e-9;
const GPU_PER_SUBMISSION: f64 = 0.15e-3;
/// The GPU is chosen only when it is this much cheaper: the estimates are rough.
const MARGIN: f64 = 2.0;

/// Where a thread's run may place strata: the CPU only, the device where it is the cheaper (the Metal binding), or
/// the device wherever it has every form (`Always`: what the device can run, apart from what it should; the
/// executor coverage gate's, and `RERAIL_DEVICE=always`'s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Binding {
    Cpu,
    Metal,
    Always,
}

thread_local! {
    /// This thread's run's binding (`typing::file_local::Binding::Metal` is `Metal`).
    static DEVICE: Cell<Binding> = const { Cell::new(Binding::Cpu) };
}

/// Whether this thread's run may use the device.
pub(in crate::code_rule_dsl) fn device() -> bool {
    DEVICE.with(Cell::get) != Binding::Cpu
}

/// Sets whether this thread's run may place strata on the device; the previous value, to restore. `RERAIL_DEVICE=always`
/// makes a Metal binding `Always`.
pub(in crate::code_rule_dsl) fn set_device(on: bool) -> bool {
    let b = match on {
        false => Binding::Cpu,
        true if always_env() => Binding::Always,
        true => Binding::Metal,
    };
    set_binding(b) != Binding::Cpu
}

/// This thread's binding.
pub(in crate::code_rule_dsl) fn binding() -> Binding {
    DEVICE.with(Cell::get)
}

/// Sets this thread's binding; the previous one, to restore.
pub(in crate::code_rule_dsl) fn set_binding(b: Binding) -> Binding {
    DEVICE.with(|d| d.replace(b))
}

fn always_env() -> bool {
    static ALWAYS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ALWAYS.get_or_init(|| std::env::var("RERAIL_DEVICE").is_ok_and(|v| v == "always"))
}

/// A stratum's shape: its forms, and its planned work.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct Shape {
    pub(in crate::code_rule_dsl) forms: BTreeSet<Form>,
    pub(in crate::code_rule_dsl) work: f64,
    pub(in crate::code_rule_dsl) rules: usize,
    /// Its join steps' estimates, when the planner recorded them (profiling, or a run that may use the device).
    pub(in crate::code_rule_dsl) joins: Vec<JoinEst>,
    /// Its rules' steps, all told: on the device, about a stage each.
    pub(in crate::code_rule_dsl) steps: usize,
    /// The rows its rules' last steps make (the planner's estimate): what the device reads back.
    pub(in crate::code_rule_dsl) rows: f64,
    /// For each form the device lacks, the operations that use it (`@sql_instr in a comparison`, `aggregate
    /// sum`): what the profile's hazards name.
    pub(in crate::code_rule_dsl) uses: BTreeMap<Form, BTreeSet<String>>,
}

/// The first call in `t` that is no row code: the operation a computed term needs (`@concat`).
fn not_row(t: &Term) -> Option<String> {
    match t {
        Term::Call(f, args) if !row_term(t) => Some(args.iter().find_map(not_row).unwrap_or_else(|| format!("@{f}"))),
        // Row code as a whole (a cat inside `str_cmp` is read from its parts).
        Term::Call(..) => None,
        _ => None,
    }
}

/// Whether a term is row code on the device: a variable, a constant, `@doc(x)` of a variable, a set of constants
/// (`@in(x, {…})`), integer arithmetic of row code (`setfix::device`'s arithmetic binds), or `str_cmp` of variables
/// and constants (the device compares the names). Anything else computes
/// a value the device has no form for.
fn row_term(t: &Term) -> bool {
    match t {
        Term::Call(f, args) if f == "doc" => matches!(args.as_slice(), [Term::Var(_)]),
        Term::Call(f, args) if f == crate::code_rule_dsl::logic::strings::SET => args.iter().all(|a| !matches!(a, Term::Call(..))),
        Term::Call(f, args) if DEVICE_ARITHMETIC.contains(&f.as_str()) && args.len() == 2 => args.iter().all(row_term),
        // The order of two symbols' names (`<` of symbols lowers to it): their texts are the device's.
        // Of names or cats' texts (compared from their parts, never made).
        Term::Call(f, args) if f == "str_cmp" && args.len() == 2 => args.iter().all(|a| !matches!(a, Term::Call(..)) || is_cat_of_rows(a) || is_text_fn_of_rows(a)),
        // A name's characters (or a cat's text's): counted over the device's names.
        Term::Call(f, args) if (f == "strlen" || f == "rr_strlen") && args.len() == 1 => !matches!(args[0], Term::Call(..)) || is_cat_of_rows(&args[0]) || is_text_fn_of_rows(&args[0]),
        // A text function the device makes into its text arena (`setfix::device`'s `TEXTFN`).
        Term::Call(f, _) if DEVICE_TEXT_FUNCTIONS.contains(&f.as_str()) => is_text_fn_of_rows(t),
        Term::Call(..) => false,
        _ => true,
    }
}

/// `cat(a, b)` (lowered, `concat`) of variables and constants: a text the device compares where it is read.
fn is_cat_of_rows(t: &Term) -> bool {
    matches!(t, Term::Call(f, args) if (f == "cat" || f == "concat") && args.len() == 2 && args.iter().all(|a| !matches!(a, Term::Call(..))))
}

/// The text functions the device makes (`metal_join::TEXTFN`), by the names the planner's terms and the native
/// engine's steps give them.
const DEVICE_TEXT_FUNCTIONS: [&str; 14] = [
    "lower", "sql_lower", "sql_upper", "head", "dir", "substr", "rr_substr", "sql_substr", "slice", "to_string", "rr_to_string", "sql_replace",
    "replace_all", "rx_replace",
];

/// A text function of variables and constants (its text argument perhaps another's value): a text the device makes.
fn is_text_fn_of_rows(t: &Term) -> bool {
    match t {
        Term::Call(f, args) if DEVICE_TEXT_FUNCTIONS.contains(&f.as_str()) && !args.is_empty() => {
            // A regex's and its replacement's texts are constants (the device's automaton is made before the run).
            let constants = !matches!(f.as_str(), "replace_all" | "rx_replace") || args[1..].iter().all(|a| !matches!(a, Term::Var(_)));
            constants && (!matches!(args[0], Term::Call(..)) || is_text_fn_of_rows(&args[0])) && args[1..].iter().all(|a| !matches!(a, Term::Call(..)))
        }
        _ => false,
    }
}

/// The first text function in `t` (`@lower`): a text the device makes into its arena, which a recursive stratum's
/// rounds and an atom's keys cannot read.
fn text_fn_in(t: &Term) -> Option<String> {
    match t {
        Term::Call(f, _) if DEVICE_TEXT_FUNCTIONS.contains(&f.as_str()) => Some(format!("@{f}")),
        Term::Call(_, args) => args.iter().find_map(text_fn_in),
        _ => None,
    }
}

/// The integer functions the device computes (`.rr`'s `+ - * / %`, Soufflé's `max` and `min`).
const DEVICE_ARITHMETIC: [&str; 7] = ["add", "sub", "mul", "div", "mod", "max", "min"];

impl Shape {
    /// Notes form `f`, and where the device lacks it, the operation that uses it.
    fn note(&mut self, f: Form, what: impl FnOnce() -> String) {
        self.forms.insert(f);
        if !DEVICE_FORMS.contains(&f) {
            self.uses.entry(f).or_default().insert(what());
        }
    }

    /// Adds one planned rule: its steps in order, its head's terms and its planned work.
    pub(in crate::code_rule_dsl) fn add(&mut self, order: &[Step], head: &[Term], work: f64, recursive: bool) {
        self.rules += 1;
        self.steps += order.len();
        self.work += work.max(0.0);
        if recursive {
            self.note(Form::Recursion, || "recursion".into());
        }
        // A cat of row terms in a head is made on the device (`mj::MAKE`), its text stored as a symbol after the run;
        // not in a recursive stratum, whose rounds would join it before it is one.
        // A text function's text, likewise (the device's arena; never in a recursive stratum).
        let arena = |t: &Term| if recursive { text_fn_in(t) } else { None };
        if let Some(op) = head.iter().filter(|t| recursive || !is_cat_of_rows(t)).find_map(|t| not_row(t).or_else(|| arena(t))) {
            self.note(Form::Computed, || format!("{op} in a head"));
        }
        for step in order {
            if let Some(m @ (Mode::Range | Mode::Climb | Mode::Descend)) = step.mode {
                self.note(Form::TreeAxis, || format!("a tree axis read as a {}", m.name()));
            }
            match &step.lit {
                Literal::Atom(a) => {
                    self.forms.insert(if a.neg { Form::Negation } else { Form::Join });
                    if a.rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) {
                        self.note(Form::TextTest, || format!("text test {}", a.rel));
                    }
                    if let Some(op) = a.args.iter().find_map(|t| not_row(t).or_else(|| text_fn_in(t))) {
                        self.note(Form::Computed, || format!("{op} in an atom's argument"));
                    }
                }
                // A text test as a comparison (`@contains(s, p) = 1`, `rx`'s search): a text test, not a value.
                Literal::Cmp { l: Term::Call(f, _), .. } | Literal::Cmp { r: Term::Call(f, _), .. } if crate::code_rule_dsl::logic::strings::TESTS.contains(&f.as_str()) => {
                    self.note(Form::TextTest, || format!("text test @{f}"));
                }
                // A cat's text compared with a name (`=`, `!=`): the device compares the names' bytes (`CATEQ`).
                // The other side a constant or a variable: a comparison of names, or (a variable no atom binds) the
                // text bound, made on the device when the head alone reads it; not in a recursive stratum.
                Literal::Cmp { l, op: "=" | "!=", r } if [(l, r), (r, l)].iter().any(|(c, o)| is_cat_of_rows(c) && match o {
                    Term::Var(v) => !recursive || order.iter().any(|s| matches!(&s.lit, Literal::Atom(a) if !a.neg && a.args.iter().any(|t| matches!(t, Term::Var(w) if w == v)))),
                    Term::Call(..) => false,
                    _ => true,
                }) => {
                    self.note(Form::Comparison, String::new);
                }
                Literal::Cmp { l, r, .. } => match [l, r].iter().find_map(|t| not_row(t).or_else(|| arena(t))) {
                    Some(op) => self.note(Form::Computed, || format!("{op} in a comparison")),
                    None => self.note(Form::Comparison, String::new),
                },
                Literal::Agg { op, atom, .. } => {
                    match op {
                        AggOp::Join { .. } => self.note(Form::StringJoin, || "string join".into()),
                        op => self.note(Form::Aggregate, || format!("aggregate {}", format!("{op:?}").to_lowercase())),
                    }
                    if atom.rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) {
                        self.note(Form::TextTest, || format!("text test {} in an aggregate", atom.rel));
                    }
                }
                Literal::Nearest { .. } => self.note(Form::Nearest, || "nearest".into()),
                Literal::Ast { .. } => self.note(Form::Matcher, || "an ast matcher".into()),
            }
        }
    }

    /// The operations behind the forms the device lacks, each `form: operation`.
    pub(in crate::code_rule_dsl) fn lacking(&self) -> Vec<String> {
        self.uses.iter().flat_map(|(f, ops)| ops.iter().map(move |o| format!("{}: {o}", f.name()))).collect()
    }
}

impl Shape {
    /// Whether the device has every form the stratum uses.
    #[cfg(test)]
    fn missing_none(&self) -> bool {
        self.forms.iter().all(|f| DEVICE_FORMS.contains(f))
    }

    /// Adds a planned rule's join steps: `order` its steps, `plan` the planner's record of them (one per step, its
    /// access and rows out), `card` a relation's rows.
    pub(in crate::code_rule_dsl) fn add_joins(&mut self, order: &[Step], plan: &crate::code_rule_dsl::logic::profile::RulePlan, card: impl Fn(&str) -> f64) {
        let mut input = 1.0;
        for (step, priced) in order.iter().zip(&plan.steps) {
            if let Literal::Atom(a) = &step.lit
                && !a.neg
            {
                // "lookup on (c0, c1)", "membership on (c0)", "range in document on (c0)"; a scan has no key.
                let keys = priced.access.split_once('(').map_or(0, |(_, cols)| cols.split(',').count());
                if keys > 0 {
                    self.joins.push(JoinEst { rel: a.rel.to_string(), input, relation: card(&a.rel), output: priced.rows, keys, range: step.mode == Some(Mode::Range) });
                }
            }
            input = priced.rows;
        }
        self.rows += input.max(0.0);
    }
}

/// Prices a join step on each form. `sorted`: the stratum sorted its relation already (a sort and probe of the same
/// relation and keys pays no sort).
pub(in crate::code_rule_dsl) fn price_join(j: &JoinEst, sorted: bool) -> JoinChoice {
    let index_s = CPU_PER_WORK * (j.input + j.output);
    let keys = j.keys as f64;
    let probe = |sorted: bool| {
        let sort = if sorted { 0.0 } else { SORT_FIXED + j.relation * (keys * UPLOAD_PER_VALUE + SORT_PER_ROW) };
        PROBE_FIXED + j.input * (keys * UPLOAD_PER_VALUE + PROBE_PER_ROW) + j.output * PROBE_PER_PAIR + sort
    };
    let probe_s = (!j.range).then(|| probe(sorted));
    let tile_s = j.range.then(|| TILE_FIXED + (j.input + j.relation) * keys * TILE_HOST_PER_VALUE + j.output * TILE_COMPARES_PER_PAIR * TILE_PER_COMPARE);
    let mut form = JoinForm::Index;
    let mut best = index_s;
    for (f, p) in [(JoinForm::Probe, probe_s), (JoinForm::Tile, tile_s)] {
        if let Some(p) = p.filter(|&p| p * MARGIN < best) {
            (form, best) = (f, p);
        }
    }
    JoinChoice { rel: j.rel.clone(), form, index_s, probe_s, tile_s }
}

/// Where a native stratum ran, as the executor coverage gate reads it (`plan::coverage`): the device, or the CPU and
/// why (the forms the device lacks, else its refusal). Kept only while `recording` is on, for every thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Ran {
    pub(in crate::code_rule_dsl) device: bool,
    pub(in crate::code_rule_dsl) why: String,
}

static RECORDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static RECORDED: std::sync::Mutex<Vec<(String, Ran)>> = std::sync::Mutex::new(Vec::new());

/// While the guard lives, every native stratum's placement is kept (`recorded`).
#[cfg(test)]
pub(in crate::code_rule_dsl) fn recording() -> impl Drop {
    struct On;
    impl Drop for On {
        fn drop(&mut self) {
            RECORDING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    RECORDING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    On
}

/// The kept placements of strata whose key starts with `prefix` (taken).
#[cfg(test)]
pub(in crate::code_rule_dsl) fn recorded(prefix: &str) -> Vec<(String, Ran)> {
    let mut all = RECORDED.lock().unwrap_or_else(|e| e.into_inner());
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *all).into_iter().partition(|(k, _)| k.starts_with(prefix));
    *all = rest;
    mine
}

/// Keeps a native stratum's placement while recording: `refused`, the device's report's refusal (`Some(None)`: it ran
/// there).
pub(in crate::code_rule_dsl) fn record(key: &str, placed: &Placement, refused: Option<Option<String>>) {
    {
        use crate::code_rule_dsl::logic::profile;
        let (ran, device): (&'static str, Vec<String>) = match (placed.gpu, &refused) {
            (true, Some(None)) => ("Metal", Vec::new()),
            (true, Some(Some(why))) => ("native CPU", vec![format!("refused at run time: {why}")]),
            (true, None) => ("native CPU", vec!["placed, no device report".into()]),
            (false, _) if !placed.lacking.is_empty() => ("native CPU", placed.lacking.clone()),
            (false, _) if !placed.device => ("native CPU", vec!["the run's binding is the CPU".into()]),
            (false, _) => ("native CPU", vec!["price: the CPU is cheaper".into()]),
        };
        profile::executor(key, ran, None, device);
    }
    if RECORDING.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return;
    }
    let ran = match (placed.gpu, refused) {
        (true, Some(None)) => Ran { device: true, why: "device".into() },
        (true, Some(Some(why))) => Ran { device: false, why: format!("refused: {why}") },
        (true, None) => Ran { device: false, why: "placed, no device report".into() },
        (false, _) if !placed.missing.is_empty() => Ran { device: false, why: format!("no {}", placed.missing.iter().map(|f| f.name()).collect::<Vec<_>>().join(", no ")) },
        (false, _) => Ran { device: false, why: placed.why() },
    };
    RECORDED.lock().unwrap_or_else(|e| e.into_inner()).push((key.to_owned(), ran));
}

/// Keeps a stratum that fell back to SQLite while recording: the class of literal the native engine had no step
/// for, and its explanation.
/// `lacking`: the device's hazards for the stratum besides (`Shape::lacking`).
pub(in crate::code_rule_dsl) fn record_fallback(key: &str, class: &str, why: &str, lacking: Vec<String>) {
    crate::code_rule_dsl::logic::profile::executor(key, "SQLite", Some(format!("{class} ({why})")), lacking);
    if RECORDING.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return;
    }
    RECORDED.lock().unwrap_or_else(|e| e.into_inner()).push((key.to_owned(), Ran { device: false, why: format!("sqlite: {class} ({why})") }));
}

/// Whether this thread's run may place strata on the device.
pub(in crate::code_rule_dsl) fn device_on() -> bool {
    device()
}

/// Where a stratum runs, and what the choice weighed.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Placement {
    /// Whether it runs on the device.
    pub(in crate::code_rule_dsl) gpu: bool,
    /// Whether the GPU's estimate is the cheaper by `MARGIN`.
    pub(in crate::code_rule_dsl) gpu_cheaper: bool,
    /// The forms the device lacks (none when it has them all).
    pub(in crate::code_rule_dsl) missing: Vec<Form>,
    /// The estimates, in seconds.
    pub(in crate::code_rule_dsl) cpu_s: f64,
    pub(in crate::code_rule_dsl) gpu_s: f64,
    /// Whether the run may use the device at all.
    pub(in crate::code_rule_dsl) device: bool,
    /// Each join step's prices and form, in the stratum's order: the form each would take on the device.
    pub(in crate::code_rule_dsl) joins: Vec<JoinChoice>,
    /// The operations behind `missing` (`Shape::lacking`).
    pub(in crate::code_rule_dsl) lacking: Vec<String>,
}

impl Placement {
    /// Why, in a few words: for the profile.
    pub(in crate::code_rule_dsl) fn why(&self) -> String {
        match (self.gpu, self.gpu_cheaper, self.missing.is_empty(), self.device) {
            (true, _, _, _) => "GPU: cheaper, and the device has every form".to_owned(),
            (false, false, _, _) => "CPU: cheaper".to_owned(),
            (false, true, true, false) => "CPU: the GPU would be cheaper, but the run's binding is the CPU".to_owned(),
            (false, true, _, _) => format!("CPU: the GPU would be cheaper, but it has no {}", self.missing.iter().map(|f| f.name()).collect::<Vec<_>>().join(", no ")),
        }
    }
}

/// The GPU's price, in seconds, of `runs` submissions doing `work` units in all: what the work a stratum did would
/// have cost on the device (the profile's check on the planned estimate).
pub(in crate::code_rule_dsl) fn gpu_price(runs: u64, work: f64) -> f64 {
    GPU_PER_SUBMISSION * runs as f64 + GPU_PER_WORK * work
}

/// Places a stratum of `shape`. Its join steps are priced on each form (`price_join`), a relation sorted once for
/// every probe of it by the same keys: the form each would take, for the profile. The stratum's GPU estimate is what
/// the device's run of it costs (`setfix::device`): one submission, its rules and stages encoded, each relation it
/// reads by keys sorted once, the joins' device work (left rows searched, pairs made, rows sorted), the rest of its
/// work at the GPU's unit price, and its rows read back. The stratum's floor (planning, opening, closing) is paid
/// wherever its joins run, so the choice weighs the joins alone: the GPU when its part is cheaper by `MARGIN`.
pub(in crate::code_rule_dsl) fn place(shape: &Shape) -> Placement {
    let cpu_s = CPU_PER_STRATUM + CPU_PER_WORK * shape.work;
    let mut sorted: BTreeSet<(&str, usize)> = BTreeSet::new();
    let joins: Vec<JoinChoice> = shape.joins.iter().map(|j| {
        let c = price_join(j, sorted.contains(&(j.rel.as_str(), j.keys)));
        if c.form == JoinForm::Probe {
            sorted.insert((j.rel.as_str(), j.keys));
        }
        c
    }).collect();
    let join_work: f64 = shape.joins.iter().map(|j| j.input + j.output).sum();
    let mut sorts: BTreeSet<(&str, usize)> = BTreeSet::new();
    let mut device_joins = 0.0;
    for j in &shape.joins {
        let keys = j.keys as f64;
        device_joins += j.input * (keys * UPLOAD_PER_VALUE + PROBE_PER_ROW) + j.output.min(1e12) * PROBE_PER_PAIR;
        if sorts.insert((j.rel.as_str(), j.keys)) {
            device_joins += DEVICE_SORT + j.relation * (keys * UPLOAD_PER_VALUE + SORT_PER_ROW);
        }
    }
    let device_s = DEVICE_SUBMISSION + DEVICE_RULE * shape.rules as f64 + DEVICE_STAGE * shape.steps as f64 + device_joins
        + GPU_PER_WORK * (shape.work - join_work).max(0.0) + DEVICE_ROW_BACK * shape.rows.min(1e12);
    let gpu_s = CPU_PER_STRATUM + device_s;
    let missing: Vec<Form> = shape.forms.iter().copied().filter(|f| !DEVICE_FORMS.contains(f)).collect();
    let binding = DEVICE.with(Cell::get);
    let device = binding != Binding::Cpu;
    let gpu_cheaper = device_s * MARGIN < CPU_PER_WORK * shape.work;
    let gpu = missing.is_empty() && (binding == Binding::Always || device && gpu_cheaper);
    Placement { gpu, gpu_cheaper, missing, cpu_s, gpu_s, device, joins, lacking: shape.lacking() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::front::Atom;

    fn atom(rel: &str, neg: bool) -> Step {
        Step { at: 0, lit: Literal::Atom(Atom { rel: rel.into(), args: vec![Term::Var("x".into())], neg, gated: false }), mode: None }
    }

    #[test]
    fn a_stratum_is_placed_by_its_price_and_named_by_the_forms_the_device_lacks() {
        // A large join with a negation, a text test and a computed head: the GPU's price is lower, and the computed
        // term is the form it lacks.
        let mut big = Shape::default();
        let len = Term::Call("sha256".into(), vec![Term::Var("x".into())]);
        big.add(&[atom("kind", false), atom("strtest_3", false), atom("within", true)], &[len], 4e6, false);
        assert_eq!(big.forms, [Form::Join, Form::Negation, Form::Computed, Form::TextTest].into_iter().collect());
        let p = place(&big);
        assert!(p.gpu_cheaper && !p.gpu, "{p:?}");
        assert_eq!(p.missing, vec![Form::Computed]);
        assert!(p.why().contains("no computed term"), "{}", p.why());
        // Without the computed head, under the Metal binding, the device takes it; under the CPU binding it does not.
        let mut plain = Shape::default();
        plain.add(&[atom("kind", false), atom("within", true)], &[Term::Var("x".into())], 4e6, false);
        assert!(plain.missing_none());
        let was = set_device(true);
        let p = place(&plain);
        set_device(was);
        assert!(p.gpu && p.missing.is_empty(), "{p:?}");
        assert!(!place(&plain).gpu, "the CPU binding keeps it");
        // `@doc(x) = @doc(y)` and `@in(x, {…})` are row code, and so is `z = cat(x, y)` (a text the device makes);
        // `sha256(x)` is not, nor a cat or `lower(x)` in a recursive stratum (its rounds would join the text before it
        // is a symbol); `lower(x)` is (a text the device makes into its arena).
        let cmp = |l: Term, r: Term| Step { at: 0, lit: Literal::Cmp { l, op: "=", r }, mode: None };
        let doc = |v: &str| Term::Call("doc".into(), vec![Term::Var(v.into())]);
        let mut docs = Shape::default();
        docs.add(&[atom("kind", false), cmp(doc("x"), doc("y"))], &[], 1.0, false);
        assert!(docs.forms.contains(&Form::Comparison) && !docs.forms.contains(&Form::Computed));
        let made = || cmp(Term::Var("z".into()), Term::Call("cat".into(), vec![Term::Var("x".into()), Term::Var("y".into())]));
        let mut cat = Shape::default();
        cat.add(&[atom("kind", false), made()], &[], 1.0, false);
        assert!(cat.forms.contains(&Form::Comparison) && !cat.forms.contains(&Form::Computed), "{:?}", cat.forms);
        let mut rec = Shape::default();
        rec.add(&[atom("kind", false), made()], &[], 1.0, true);
        assert!(rec.forms.contains(&Form::Computed));
        let lower = || cmp(Term::Var("z".into()), Term::Call("lower".into(), vec![Term::Var("x".into())]));
        let mut len = Shape::default();
        len.add(&[atom("kind", false), cmp(Term::Var("z".into()), Term::Call("sha256".into(), vec![Term::Var("x".into())]))], &[], 1.0, false);
        assert!(len.forms.contains(&Form::Computed));
        let mut low = Shape::default();
        low.add(&[atom("kind", false), lower()], &[], 1.0, false);
        assert!(!low.forms.contains(&Form::Computed), "{:?}", low.forms);
        let mut low_rec = Shape::default();
        low_rec.add(&[atom("kind", false), lower()], &[], 1.0, true);
        assert!(low_rec.forms.contains(&Form::Computed));
        // A tiny stratum stays on the CPU: the submission costs more than the work, even under the Metal binding.
        let mut small = Shape::default();
        small.add(&[atom("kind", false)], &[], 10.0, false);
        let was = set_device(true);
        let p = place(&small);
        set_device(was);
        assert!(!p.gpu_cheaper && !p.gpu);
        assert_eq!(p.why(), "CPU: cheaper");
        // Join steps priced on each form: a large equi-join takes the sort and probe, its second probe of the same
        // relation pays no sort; a range inside a document takes the tile; a small lookup stays on the CPU's index.
        let est = |rel: &str, input: f64, output: f64, range: bool| JoinEst { rel: rel.into(), input, relation: 1e6, output, keys: 1, range };
        let mut joined = Shape::default();
        joined.joins = vec![est("a", 1e6, 1e6, false), est("a", 1e6, 1e6, false), est("b", 1e6, 1e6, true), est("c", 10.0, 10.0, false)];
        let p = place(&joined);
        let forms: Vec<JoinForm> = p.joins.iter().map(|c| c.form).collect();
        assert_eq!(forms, vec![JoinForm::Probe, JoinForm::Probe, JoinForm::Tile, JoinForm::Index], "{p:?}");
        assert!(p.joins[1].probe_s < p.joins[0].probe_s, "the second probe finds the relation sorted");
        // Recursion and computed terms are forms too.
        let mut rec = Shape::default();
        rec.add(&[atom("edge", false)], &[Term::Call("add".into(), vec![])], 1.0, true);
        assert!(rec.forms.contains(&Form::Recursion) && rec.forms.contains(&Form::Computed));
    }
}

/// A measurement aid only: `RERAIL_DEVICE_JOINS=off` keeps every stratum's joins on the CPU under a device binding,
/// so a run can be paired against the same binding with them on. Placement still prices and reports as usual.
pub(crate) fn device_joins_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var("RERAIL_DEVICE_JOINS").is_ok_and(|v| v == "off"))
}
