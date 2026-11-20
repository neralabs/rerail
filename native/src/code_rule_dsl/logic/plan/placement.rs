//! Where a stratum runs, CPU or GPU, from its shape and the planner's estimates (decision 0095 §8).
//!
//! Before a stratum opens, its seeds' and probes' plans give an estimate of its work: the planner's own unit, the
//! executor's count (decision 0297: each step costs the rows it passes on). Each backend has a price for that work:
//!
//! - **CPU:** a floor per stratum run (open, its plans and replans, index building, close), a cost per row a scan
//!   reads, and a cost per other unit of work: fitted on PostHog's `posthog/hogql` after 0297 (decision 0304). A
//!   join step alone is priced by 0297's fit of the executor's CPU: per probe and per row out.
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

use super::{Mode, Order};
use crate::code_rule_dsl::logic::program_image::{AggRef, CmpOp, Lit, TermRef, Version};
use crate::code_rule_dsl::logic::device::caps::{self, DeviceFn};
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
    /// Its input and output when every row is walked (`StepPlan::all`): the CPU's walk keeps one witness past the
    /// bound head (decision 0229), the device's stages make every row (decision 0277).
    pub(in crate::code_rule_dsl) walked: (f64, f64),
    pub(in crate::code_rule_dsl) keys: usize,
    pub(in crate::code_rule_dsl) range: bool,
    /// Its lookup's CPU as a multiple of a derived relation's (`Source::lookup_cost`, decision 0312): a view that
    /// walks the tree or searches the documents costs several.
    pub(in crate::code_rule_dsl) cost: f64,
}

impl JoinEst {
    /// Its lookups' CPU on the CPU's index: per probe (a row in) and per row out, times its relation's cost.
    fn index_s(&self) -> f64 {
        self.cost * (CPU_PER_PROBE * self.input + CPU_PER_ROW_OUT * self.output)
    }
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

/// The device's stratum, as `setfix::device` runs it, in seconds of the host's CPU (decision 0325 step 6): its
/// relations read where they lie and its heads written in place, so no copy is priced. Fitted on PostHog's
/// `posthog/hogql` (2026-11-17, 1,193 runs on the GPU in three cold runs, each part of a run's host time against its
/// own terms, `placement.tsv`):
/// - A run (`DEVICE_RUN`): its share of its level's wave (one command buffer, committed and waited for once, for
///   every stratum in it: 77% of runs), the batch, and its fixed encoding. 0.18 ms. It was 0.10 ms a submission,
///   one a stratum.
/// - A stage (`DEVICE_STAGE`, a rule's step): its count, scan and scatter encoded. 30 µs. A rule adds nothing past its
///   stages (the rule's own 6 µs was dropped).
/// - An index (`DEVICE_SORT`, a relation probed by keys): its build encoded (`Batch::index`: a hash and the radix
///   passes) or found the relation's own. 65 µs.
/// - A view made (`DEVICE_VIEW`): an input the device makes a view of from the chunk's block (decision 0289), one no
///   device stratum of the run read before (`Run::device_views`; a view made stays resident), and a stratum's text
///   and verdict words: 0.4 ms, the measured average (0.43 s over 1,086 views priced as made in 962 runs). Neither
///   the views nor their values predict a stratum's share (R² under 0.3, Q67): `ann_ctx` pays 2.1 ms a run for one
///   view of 1.2 M values when it makes it, 0.5 ms when `ann_root` made it first. It was 2.2 ns a value read of
///   every relation, derived ones too, which the device now reads in place.
/// - A head row: 2 ns into the store, not priced (it was 105 ns, before the heads were written in place).
const DEVICE_RUN: f64 = 0.18e-3;
const DEVICE_STAGE: f64 = 30e-6;
const DEVICE_SORT: f64 = 65e-6;
const DEVICE_VIEW: f64 = 0.4e-3;

/// CPU: seconds per unit of planned work that is no scan's row, per row a scan reads, and per stratum run (decision
/// 0304): a least-squares fit of the CPU-placed strata of a cold run over PostHog's `posthog/hogql` (169 strata,
/// 2,395 runs, 2.5 s of CPU) against their runs, scanned rows and other units, the planned work counted as the
/// executor counts it (0297). 50 ns a unit is between 0297's 72 ns a join's row out and 21 ns a filter's row. A
/// scan's row now costs a view's row read in place (Q36's 150–300 instructions a row), not the stored copy's 30 ns
/// measured before (decision 0277). A run's floor holds its plans and replans (0297: about 1.2 ms a plan, loaded),
/// not only its open and close (0.13 ms on 2026-11-09).
pub(in crate::code_rule_dsl) const CPU_PER_WORK: f64 = 50e-9;
const CPU_PER_SCANNED: f64 = 117e-9;
pub(in crate::code_rule_dsl) const CPU_PER_STRATUM: f64 = 0.7e-3;
/// CPU: a join step's index lookup, per probe (a row in) and per row out: 0297's fit of the executor's CPU.
const CPU_PER_PROBE: f64 = 124e-9;
const CPU_PER_ROW_OUT: f64 = 72e-9;
/// GPU: seconds per unit of planned work, and per submission (assumed; see the module's comment).
const GPU_PER_WORK: f64 = 2e-9;
const GPU_PER_SUBMISSION: f64 = 0.15e-3;
/// The GPU is chosen while its price is under this many times the CPU's (decision 0321): it is the wider of the two,
/// with the more memory bandwidth, so the work it takes leaves the CPU's cores to the strata only they run. Before,
/// the GPU had to be twice the cheaper (`MARGIN`, 2), against estimates 8 times off (decision 0320).
pub(in crate::code_rule_dsl) const GPU_BIAS: f64 = 2.0;

/// `GPU_BIAS`, or `RERAIL_GPU_BIAS` when set: a measurement aid only, to refit the bias over runs at several.
pub(in crate::code_rule_dsl) fn gpu_bias() -> f64 {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_GPU_BIAS").ok().and_then(|v| v.parse().ok()).filter(|b: &f64| *b > 0.0).unwrap_or(GPU_BIAS))
}

/// Where a thread's run may place strata: the CPU only, the device where it is the cheaper (the Metal binding), or
/// the device wherever it has every form (`Always`: what the device can run, apart from what it should; the
/// executor coverage gate's, and `RERAIL_DEVICE=always`'s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    Cpu,
    Metal,
    Always,
}

/// The run's binding (`typing::file_local::Binding::Metal` is `Metal`), in the run's context: its jobs' too.
#[derive(Clone, Copy)]
struct Device(Binding);
impl Default for Device {
    fn default() -> Self {
        Device(Binding::Cpu)
    }
}

/// Whether this thread's run may use the device.
pub(in crate::code_rule_dsl) fn device() -> bool {
    binding() != Binding::Cpu
}

/// Whether this run may place strata on the device, until the guard drops. `RERAIL_DEVICE=always` makes a Metal
/// binding `Always`.
pub(in crate::code_rule_dsl) fn set_device(on: bool) -> rerail_prof::code_rule_dsl::logic::profile::Bound {
    let b = match on {
        false => Binding::Cpu,
        true if always_env() => Binding::Always,
        true => Binding::Metal,
    };
    set_binding(b)
}

/// What a block this run makes for the device to read asks of the block policy (decision 0342): the device may
/// read it when the run is bound to the device; a CPU run's never.
pub fn block_device() -> rerail_blocks::alloc::Device {
    if device() { rerail_blocks::alloc::Device::May } else { rerail_blocks::alloc::Device::No }
}

/// The run's binding.
pub fn binding() -> Binding {
    rerail_prof::code_rule_dsl::logic::profile::bound::<Device>().0
}

/// The run's binding, until the guard drops.
pub fn set_binding(b: Binding) -> rerail_prof::code_rule_dsl::logic::profile::Bound {
    rerail_prof::code_rule_dsl::logic::profile::bind(Device(b))
}

fn always_env() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_DEVICE").is_ok_and(|v| v == "always"))
}

/// A stratum's shape: its forms, and its planned work.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct Shape {
    pub(in crate::code_rule_dsl) forms: BTreeSet<Form>,
    pub(in crate::code_rule_dsl) work: f64,
    pub(in crate::code_rule_dsl) rules: usize,
    /// Its join steps' estimates, when the planner recorded them (profiling, or a run that may use the device).
    pub(in crate::code_rule_dsl) joins: Vec<JoinEst>,
    /// The relations its atoms read, each with its rows, the columns an atom reads of it (the most any one does),
    /// and whether the device would make a view of it (decision 0289): an input no device stratum of the run read
    /// before (`Run::device_views`). A relation the program derives is read in place (0325), and so is a view made.
    pub(in crate::code_rule_dsl) reads: Vec<(String, f64, usize, bool)>,
    /// The rows its scans read (a positive atom read whole), of its planned work: on the CPU a scanned row costs a
    /// read, not a probe (`CPU_PER_SCANNED`).
    pub(in crate::code_rule_dsl) scanned: f64,
    /// Its rules' steps, all told: on the device, about a stage each.
    pub(in crate::code_rule_dsl) steps: usize,
    /// The rows its rules' last steps make (the planner's estimate): what the device reads back.
    pub(in crate::code_rule_dsl) rows: f64,
    /// For each form the device lacks, the operations that use it (`@sql_instr in a comparison`, `aggregate
    /// sum`): what the profile's hazards name.
    pub(in crate::code_rule_dsl) uses: BTreeMap<Form, BTreeSet<String>>,
    /// The rows its seeds make, by the relation they make (the planner's estimates): a recursive relation's first
    /// delta.
    pub(in crate::code_rule_dsl) seeds: BTreeMap<String, f64>,
    /// Its recursive probes, each with its delta relation, its planned work and rows, and the delta its plan was
    /// priced at: totalled over every row the relation gains (`close_rounds`).
    pub(in crate::code_rule_dsl) probes: Vec<(String, f64, f64, f64)>,
}

/// A call's name and arguments.
fn call<'v, 't>(v: &'v Version, t: TermRef<'t>) -> Option<(&'v str, Vec<TermRef<'t>>)> {
    match t {
        TermRef::Call(f, args) => Some((v.text(f), args.collect())),
        _ => None,
    }
}

fn is_call(t: &TermRef<'_>) -> bool {
    matches!(t, TermRef::Call(..))
}

/// The first call in `t` that is no row code: the operation a computed term needs (`@concat`).
fn not_row(v: &Version, t: TermRef<'_>) -> Option<String> {
    let (f, args) = call(v, t)?;
    // Row code as a whole (a cat inside `str_cmp` is read from its parts).
    (!row_term(v, t)).then(|| args.iter().find_map(|&a| not_row(v, a)).unwrap_or_else(|| format!("@{f}")))
}

/// Whether a term is row code on the device: a variable, a constant, `@doc(x)` of a variable, a set of constants
/// (`@in(x, {…})`), integer arithmetic of row code (`setfix::device`'s arithmetic binds), or `str_cmp` of variables
/// and constants (the device compares the names). Anything else computes
/// a value the device has no form for.
fn row_term(v: &Version, t: TermRef<'_>) -> bool {
    let Some((f, args)) = call(v, t) else { return true };
    match f {
        "doc" => matches!(args.as_slice(), [TermRef::Var(_)]),
        f if f == crate::code_rule_dsl::logic::strings::SET => args.iter().all(|a| !is_call(a)),
        // The device's functions (`device::caps`, the table its compile reads).
        f => match device_fn(f) {
            Some(d) if d.arity() != args.len() => false,
            Some(DeviceFn::Arith(_)) => args.iter().all(|&a| row_term(v, a)),
            // The order of two symbols' names (`<` of symbols lowers to it), or a name's characters: of names or
            // cats' texts (compared from their parts, never made), or of texts the device makes.
            Some(DeviceFn::Names { .. }) => args.iter().all(|&a| !is_call(&a) || is_cat_of_rows(v, a) || is_text_fn_of_rows(v, a)),
            // A text function the device makes into its text arena (`setfix::device`'s `TEXTFN`).
            Some(DeviceFn::Text { .. } | DeviceFn::RegexReplace) => is_text_fn_of_rows(v, t),
            None => false,
        },
    }
}

/// The device's form of `.rr`'s (or the native engine's) function `f`.
fn device_fn(f: &str) -> Option<DeviceFn> {
    caps::device_fn(caps::native(f))
}

/// Whether the device makes `f`'s value into its text arena.
fn makes_text(f: &str) -> bool {
    device_fn(f).is_some_and(DeviceFn::makes_text)
}

/// `cat(a, b)` (lowered, `concat`) of variables and constants: a text the device compares where it is read.
fn is_cat_of_rows(v: &Version, t: TermRef<'_>) -> bool {
    call(v, t).is_some_and(|(f, args)| (f == "cat" || f == "concat") && args.len() == 2 && args.iter().all(|a| !is_call(a)))
}

/// `cat(a, b)` whose parts are variables, constants, texts the device makes (`is_text_fn_of_rows`) or such cats (a
/// three-part `@concat` lowers to two): a text the device makes per row into its arena (`mj::ROW_CONCAT`).
fn is_made_cat(v: &Version, t: TermRef<'_>) -> bool {
    call(v, t).is_some_and(|(f, args)| {
        (f == "cat" || f == "concat") && args.len() == 2 && args.iter().all(|&a| !is_call(&a) || is_text_fn_of_rows(v, a) || is_made_cat(v, a))
    })
}

/// A text function of variables and constants (its text argument perhaps another's value): a text the device makes.
fn is_text_fn_of_rows(v: &Version, t: TermRef<'_>) -> bool {
    match call(v, t) {
        Some((f, args)) if makes_text(f) && !args.is_empty() => {
            // A regex's and its replacement's texts are constants (the device's automaton is made before the run).
            let constants = device_fn(f) != Some(DeviceFn::RegexReplace) || args[1..].iter().all(|a| !matches!(a, TermRef::Var(_)));
            constants && (!is_call(&args[0]) || is_text_fn_of_rows(v, args[0])) && args[1..].iter().all(|a| !is_call(a))
        }
        _ => false,
    }
}

/// The first text function the device makes in `t` (its arena), the operation it needs.
fn text_fn_in(v: &Version, t: TermRef<'_>) -> Option<String> {
    let (f, args) = call(v, t)?;
    if makes_text(f) { Some(format!("@{f}")) } else { args.iter().find_map(|&a| text_fn_in(v, a)) }
}

impl Shape {
    /// Notes form `f`, and where the device lacks it, the operation that uses it.
    fn note(&mut self, f: Form, what: impl FnOnce() -> String) {
        self.forms.insert(f);
        if !DEVICE_FORMS.contains(&f) {
            self.uses.entry(f).or_default().insert(what());
        }
    }

    /// Seeds' rows `rows` made into `rel`.
    pub(in crate::code_rule_dsl) fn seeded(&mut self, rel: &str, rows: f64) {
        *self.seeds.entry(rel.to_owned()).or_default() += rows.max(0.0);
    }

    /// A recursive probe of delta `rel`, planned at `work` and `rows` for a delta of `delta` rows.
    pub(in crate::code_rule_dsl) fn probe(&mut self, rel: &str, work: f64, rows: f64, delta: f64) {
        self.probes.push((rel.to_owned(), work.max(0.0), rows.max(0.0), delta));
    }

    /// The probes' work over the stratum's rounds, as the executor counts it (decision 0304 §3): each row a
    /// recursive relation gains is a delta once, and each probe of it reads every such row. A probe's plan was priced
    /// at the delta the planner saw (one round's, often the relation's few rows when it opened); the rows its
    /// relation gains are at least its seeds' rows, and the rows its probes make from them: one round more, at most
    /// as many again (a round's estimate is rough, and its rounds after that compound its error).
    pub(in crate::code_rule_dsl) fn close_rounds(&mut self) {
        let mut by: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
        for (rel, work, rows, delta) in &self.probes {
            let d = delta.max(1.0);
            let at = by.entry(rel.as_str()).or_default();
            at.0 += work / d;
            at.1 += rows / d;
        }
        for (rel, (per_row, out_per_row)) in by {
            let first = self.probes.iter().filter(|p| p.0 == rel).map(|p| p.3).fold(0.0, f64::max).max(self.seeds.get(rel).copied().unwrap_or(0.0)).max(1.0);
            self.work += per_row * first * (1.0 + out_per_row.min(1.0));
        }
        self.probes.clear();
    }

    /// Adds one planned rule: its steps in order (its head's terms the rule's) and its planned work.
    /// `texts_kept`: its head is a `shortlex_min` lattice, whose value its recursive rounds make into one device arena
    /// (decision 0307).
    pub(in crate::code_rule_dsl) fn add(&mut self, order: Order<'_, '_>, work: f64, recursive: bool, texts_kept: bool) {
        let (b, v) = (order.b, order.b.v);
        self.rules += 1;
        self.steps += order.len();
        self.work += work.max(0.0);
        if recursive {
            self.note(Form::Recursion, || "recursion".into());
        }
        // A cat of row terms in a head is made on the device (`mj::MAKE`), its text stored as a symbol after the run;
        // not in a recursive stratum, whose rounds would join it before it is one, unless the stratum keeps its texts
        // in one arena (0307: the device's compile takes them into the lattice's value only).
        // A text function's text, likewise (the device's arena).
        let recursive = recursive && !texts_kept;
        let arena = |t: TermRef<'_>| if recursive { text_fn_in(v, t) } else { None };
        if let Some(op) = b.head().args().filter(|&t| recursive || !is_cat_of_rows(v, t)).find_map(|t| not_row(v, t).or_else(|| arena(t))) {
            self.note(Form::Computed, || format!("{op} in a head"));
        }
        let tests = |t: TermRef<'_>| call(v, t).map(|(f, _)| f).filter(|f| crate::code_rule_dsl::logic::strings::TESTS.contains(f));
        for step in order.steps {
            if let Some(m @ (Mode::Range | Mode::Climb | Mode::Descend)) = step.mode {
                self.note(Form::TreeAxis, || format!("a tree axis read as a {}", m.name()));
            }
            match order.view(step) {
                Lit::Atom(a) => {
                    let rel = b.rel(a);
                    self.forms.insert(if a.neg() { Form::Negation } else { Form::Join });
                    if rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) {
                        self.note(Form::TextTest, || format!("text test {rel}"));
                    }
                    if let Some(op) = a.args().find_map(|t| not_row(v, t).or_else(|| text_fn_in(v, t))) {
                        self.note(Form::Computed, || format!("{op} in an atom's argument"));
                    }
                }
                // A text test as a comparison (`@contains(s, p) = 1`, `rx`'s search): a text test, not a value.
                Lit::Cmp { l, r, .. } if let Some(f) = tests(l).or_else(|| tests(r)) => {
                    self.note(Form::TextTest, || format!("text test @{f}"));
                }
                // A cat's text compared with a name (`=`, `!=`): the device compares the names' bytes (`CATEQ`).
                // The other side a constant or a variable: a comparison of names, or (a variable no atom binds) the
                // text bound, made on the device when the head alone reads it; not in a recursive stratum, unless it
                // keeps its texts (0307). A cat of made texts (`is_made_cat`) binds a variable only.
                Lit::Cmp { l, op: CmpOp::Eq | CmpOp::Ne, r }
                    if [(l, r), (r, l)].into_iter().any(|(c, o)| {
                        (is_cat_of_rows(v, c) || matches!(o, TermRef::Var(_)) && is_made_cat(v, c))
                            && match o {
                                TermRef::Var(x) => {
                                    !recursive || order.steps.iter().any(|s| b.plain_atom(s.at).is_some_and(|a| !a.neg() && a.args().any(|t| t.var() == Some(x))))
                                }
                                TermRef::Call(..) => false,
                                _ => true,
                            }
                    }) =>
                {
                    self.note(Form::Comparison, String::new);
                }
                Lit::Cmp { l, r, .. } => match [l, r].into_iter().find_map(|t| not_row(v, t).or_else(|| arena(t))) {
                    Some(op) => self.note(Form::Computed, || format!("{op} in a comparison")),
                    None => self.note(Form::Comparison, String::new),
                },
                Lit::Agg { op, atom, .. } => {
                    match op {
                        AggRef::Join { .. } => self.note(Form::StringJoin, || "string join".into()),
                        op => {
                            let name = match op {
                                AggRef::Count => "count",
                                AggRef::Min => "min",
                                AggRef::Max => "max",
                                AggRef::Sum => "sum",
                                _ => "product",
                            };
                            self.note(Form::Aggregate, || format!("aggregate {name}"))
                        }
                    }
                    let rel = b.rel(atom);
                    if rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) {
                        self.note(Form::TextTest, || format!("text test {rel} in an aggregate"));
                    }
                }
                Lit::Nearest { .. } => self.note(Form::Nearest, || "nearest".into()),
                Lit::Ast { .. } => self.note(Form::Matcher, || "an ast matcher".into()),
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
    #[cfg(any(test, feature = "test-hooks"))]
    fn missing_none(&self) -> bool {
        self.forms.iter().all(|f| DEVICE_FORMS.contains(f))
    }

    /// Adds a planned rule's join steps: `order` its steps, `plan` the planner's record of them (one per step, its
    /// access and rows out), `card` a relation's rows, `cost` a lookup on a relation's column (`Stats::lookup_cost`).
    /// `is_input` whether a relation is one of the program's inputs (a view).
    pub(in crate::code_rule_dsl) fn add_joins(&mut self, order: Order<'_, '_>, plan: &crate::code_rule_dsl::logic::profile::RulePlan, card: impl Fn(&str) -> f64, cost: impl Fn(&str, &str) -> f64, is_input: impl Fn(&str) -> bool) {
        let (mut input, mut walked) = (1.0, 1.0);
        for (step, priced) in order.steps.iter().zip(&plan.steps) {
            if let Some(a) = order.b.plain_atom(step.at) {
                let rel = order.b.rel(a);
                let cols = a.args().filter(|t| !matches!(t, TermRef::Wild)).count();
                match self.reads.iter_mut().find(|r| r.0 == rel) {
                    Some(r) => r.2 = r.2.max(cols),
                    None => self.reads.push((rel.to_string(), card(rel), cols, is_input(rel))),
                }
            }
            if let Some(a) = order.b.plain_atom(step.at)
                && !a.neg()
            {
                let rel = order.b.rel(a);
                // "lookup on (c0, c1)", "membership on (c0)", "range in document on (c0)"; a scan has no key.
                let cols: Vec<&str> = priced.access.split_once('(').map_or(Vec::new(), |(_, cols)| cols.trim_end_matches(')').split(',').map(str::trim).collect());
                let keys = cols.len();
                if keys == 0 {
                    self.scanned += card(rel);
                }
                if keys > 0 {
                    // A lookup by several columns probes its first one and filters by the rest.
                    let cost = cost(rel, cols[0]);
                    self.joins.push(JoinEst { rel: rel.to_string(), input, relation: card(rel), output: priced.rows, walked: (walked, priced.all), keys, range: step.mode == Some(Mode::Range), cost });
                }
            }
            input = priced.rows;
            walked = priced.all;
        }
        self.rows += input.max(0.0);
    }
}

/// Prices a join step on each form. `sorted`: the stratum sorted its relation already (a sort and probe of the same
/// relation and keys pays no sort).
pub(in crate::code_rule_dsl) fn price_join(j: &JoinEst, sorted: bool) -> JoinChoice {
    let index_s = j.index_s();
    let keys = j.keys as f64;
    let probe = |sorted: bool| {
        let sort = if sorted { 0.0 } else { SORT_FIXED + j.relation * (keys * UPLOAD_PER_VALUE + SORT_PER_ROW) };
        PROBE_FIXED + j.input * (keys * UPLOAD_PER_VALUE + PROBE_PER_ROW) + j.output * PROBE_PER_PAIR + sort
    };
    let probe_s = (!j.range).then(|| probe(sorted));
    let tile_s = j.range.then(|| TILE_FIXED + (j.input + j.relation) * keys * TILE_HOST_PER_VALUE + j.output * TILE_COMPARES_PER_PAIR * TILE_PER_COMPARE);
    // The device's form is the cheaper of its own, and is taken over the CPU's index while under `GPU_BIAS` times it.
    let device = [(JoinForm::Probe, probe_s), (JoinForm::Tile, tile_s)].into_iter()
        .filter_map(|(f, p)| Some((f, p?))).min_by(|a, b| a.1.total_cmp(&b.1));
    let form = match device {
        Some((f, p)) if p < index_s * gpu_bias() => f,
        _ => JoinForm::Index,
    };
    JoinChoice { rel: j.rel.clone(), form, index_s, probe_s, tile_s }
}

/// Where a native stratum ran, as the executor coverage gate reads it (`plan::coverage`): the device, or the CPU and
/// why (the forms the device lacks, else its refusal). Kept only while `recording` is on, for every thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    pub device: bool,
    pub why: String,
}

/// The placements kept while `recording` (decision 0442: the recording caller's, in the run's context, so its runs'
/// jobs record into it too).
type Recorded = std::sync::Arc<std::sync::Mutex<Vec<(String, Ran)>>>;

#[derive(Clone, Default)]
struct Recording(Option<Recorded>);

/// The recording under way for this caller's runs, if one is.
fn recorded_here() -> Option<Recorded> {
    rerail_prof::code_rule_dsl::logic::profile::bound::<Recording>().0
}

/// While the guard lives, every native stratum's placement in this caller's runs is kept (`recorded`).
#[cfg(any(test, feature = "test-hooks"))]
pub fn recording() -> Option<rerail_prof::code_rule_dsl::logic::profile::Bound> {
    recorded_here().is_none().then(|| rerail_prof::code_rule_dsl::logic::profile::bind(Recording(Some(Default::default()))))
}

/// The kept placements of strata whose key starts with `prefix` (taken).
#[cfg(any(test, feature = "test-hooks"))]
pub fn recorded(prefix: &str) -> Vec<(String, Ran)> {
    let Some(all) = recorded_here() else { return Vec::new() };
    let mut all = all.lock().unwrap_or_else(|e| e.into_inner());
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
    let Some(recorded) = recorded_here() else { return };
    let ran = match (placed.gpu, refused) {
        (true, Some(None)) => Ran { device: true, why: "device".into() },
        (true, Some(Some(why))) => Ran { device: false, why: format!("refused: {why}") },
        (true, None) => Ran { device: false, why: "placed, no device report".into() },
        (false, _) if !placed.missing.is_empty() => Ran { device: false, why: format!("no {}", placed.missing.iter().map(|f| f.name()).collect::<Vec<_>>().join(", no ")) },
        (false, _) => Ran { device: false, why: placed.why() },
    };
    recorded.lock().unwrap_or_else(|e| e.into_inner()).push((key.to_owned(), ran));
}

/// Keeps a stratum that fell back to SQLite while recording: the class of literal the native engine had no step
/// for, and its explanation.
/// `lacking`: the device's hazards for the stratum besides (`Shape::lacking`).
pub(in crate::code_rule_dsl) fn record_fallback(key: &str, class: &str, why: &str, lacking: Vec<String>) {
    crate::code_rule_dsl::logic::profile::executor(key, "SQLite", Some(format!("{class} ({why})")), lacking);
    let Some(recorded) = recorded_here() else { return };
    recorded.lock().unwrap_or_else(|e| e.into_inner()).push((key.to_owned(), Ran { device: false, why: format!("sqlite: {class} ({why})") }));
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
    /// Whether the GPU's estimate is under `GPU_BIAS` times the CPU's.
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
    /// The device price's terms, as priced: what the profile fits the device's prices on (`placement.tsv`).
    pub(in crate::code_rule_dsl) terms: DeviceTerms,
}

/// The terms of a stratum's device price, before their prices: its rules and steps (a stage each), the relations it
/// sorts or indexes by keys, the values its atoms read (rows by the columns read), the rows its rules make, its joins'
/// device work (seconds, at the relational lab's prices) and its other planned work.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::code_rule_dsl) struct DeviceTerms {
    pub(in crate::code_rule_dsl) rules: f64,
    pub(in crate::code_rule_dsl) steps: f64,
    pub(in crate::code_rule_dsl) sorts: f64,
    pub(in crate::code_rule_dsl) values: f64,
    pub(in crate::code_rule_dsl) rows: f64,
    pub(in crate::code_rule_dsl) joins_s: f64,
    pub(in crate::code_rule_dsl) other_work: f64,
    /// The views of inputs it makes: those no device stratum of the run made before (`Shape::reads`).
    pub(in crate::code_rule_dsl) views: f64,
}

impl DeviceTerms {
    /// In the profile's order (`PlacedTotal::gpu_terms`).
    pub(in crate::code_rule_dsl) fn array(&self) -> [f64; 8] {
        [self.rules, self.steps, self.sorts, self.values, self.rows, self.joins_s, self.other_work, self.views]
    }
}

impl Placement {
    /// Why, in a few words: for the profile.
    pub(in crate::code_rule_dsl) fn why(&self) -> String {
        match (self.gpu, self.gpu_cheaper, self.missing.is_empty(), self.device) {
            (true, _, _, _) => "GPU: its price within the GPU's bias, and the device has every form".to_owned(),
            (false, false, _, _) => "CPU: cheaper".to_owned(),
            (false, true, true, false) => "CPU: the GPU's price is within its bias, but the run's binding is the CPU".to_owned(),
            (false, true, _, _) => format!("CPU: the GPU's price is within its bias, but it has no {}", self.missing.iter().map(|f| f.name()).collect::<Vec<_>>().join(", no ")),
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
/// the device's run of it costs (`setfix::device`): its run (its share of its wave), its stages encoded, each relation
/// it reads by keys indexed once, each input it reads (a view made), the joins' device work (left rows searched,
/// pairs made, rows sorted), and the rest of its work at the GPU's unit price; no copies (decision 0325 step 6). The
/// stratum's floor (planning, opening, closing) is paid
/// wherever its joins run, so the choice weighs the joins alone: the GPU while its part is under `GPU_BIAS` times the CPU's.
pub(in crate::code_rule_dsl) fn place(shape: &Shape) -> Placement {
    let scanned = shape.scanned.min(shape.work).max(0.0);
    // A lookup into a view that costs more than a derived relation's adds what it costs past one (decision 0312):
    // the units of planned work are priced as the fit's mix, at about one.
    let views: f64 = shape.joins.iter().filter(|j| j.cost > 1.0).map(|j| j.index_s() * (1.0 - 1.0 / j.cost)).sum();
    let cpu_work_s = CPU_PER_WORK * (shape.work - scanned) + CPU_PER_SCANNED * scanned + views;
    let cpu_s = CPU_PER_STRATUM + cpu_work_s;
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
        // The device walks every row (no witness cut): its input and output uncut.
        let (input, output) = j.walked;
        device_joins += input.min(1e12) * (keys * UPLOAD_PER_VALUE + PROBE_PER_ROW) + output.min(1e12) * PROBE_PER_PAIR;
        if sorts.insert((j.rel.as_str(), j.keys)) {
            device_joins += j.relation * (keys * UPLOAD_PER_VALUE + SORT_PER_ROW);
        }
    }
    let terms = DeviceTerms {
        rules: shape.rules as f64,
        steps: shape.steps as f64,
        sorts: sorts.len() as f64,
        values: shape.reads.iter().filter(|r| r.3).map(|(_, rows, cols, _)| rows * *cols as f64).sum::<f64>().min(1e12),
        rows: shape.rows.min(1e12),
        joins_s: device_joins,
        other_work: (shape.work - join_work).max(0.0),
        views: shape.reads.iter().filter(|r| r.3).count() as f64,
    };
    let device_s = DEVICE_RUN + DEVICE_STAGE * terms.steps + DEVICE_SORT * terms.sorts + DEVICE_VIEW * terms.views + terms.joins_s
        + GPU_PER_WORK * terms.other_work;
    let gpu_s = CPU_PER_STRATUM + device_s;
    let missing: Vec<Form> = shape.forms.iter().copied().filter(|f| !DEVICE_FORMS.contains(f)).collect();
    let binding = binding();
    let device = binding != Binding::Cpu;
    // The GPU while its price is under `GPU_BIAS` times the CPU's (decision 0321).
    let gpu_cheaper = device_s < cpu_work_s * gpu_bias();
    let gpu = missing.is_empty() && (binding == Binding::Always || device && gpu_cheaper);
    Placement { gpu, gpu_cheaper, missing, cpu_s, gpu_s, device, joins, lacking: shape.lacking(), terms }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::front::{Atom, Literal, Rule, Term};
    use crate::code_rule_dsl::logic::plan::Step;

    fn atom(rel: &str, neg: bool) -> Literal {
        Literal::Atom(Atom { rel: rel.into(), args: [Term::Var("x".into())].into(), neg, gated: false })
    }

    /// Adds `lits`, planned in their order, to `shape`.
    fn adds(shape: &mut Shape, lits: &[Literal], head: &[Term], work: f64, recursive: bool) {
        let steps: Vec<Step> = (0..lits.len()).map(|at| Step { at, mode: None }).collect();
        let head = Atom { rel: "out".into(), args: head.to_vec().into(), neg: false, gated: false };
        let made = crate::code_rule_dsl::logic::plan::body::alone(&Rule { head, body: lits.to_vec().into() }).expect("a rule");
        let b = crate::code_rule_dsl::logic::plan::body::nth(&made, 0);
        shape.add(Order::new(&b, &steps), work, recursive, false);
    }

    #[test]
    fn a_stratum_is_placed_by_its_price_and_named_by_the_forms_the_device_lacks() {
        // A large join with a negation, a text test and a computed head: the GPU's price is lower, and the computed
        // term is the form it lacks.
        let mut big = Shape::default();
        let len = Term::Call("sha256".into(), [Term::Var("x".into())].into());
        adds(&mut big, &[atom("kind", false), atom("strtest_3", false), atom("within", true)], &[len], 4e6, false);
        assert_eq!(big.forms, [Form::Join, Form::Negation, Form::Computed, Form::TextTest].into_iter().collect());
        let p = place(&big);
        assert!(p.gpu_cheaper && !p.gpu, "{p:?}");
        assert_eq!(p.missing, vec![Form::Computed]);
        assert!(p.why().contains("no computed term"), "{}", p.why());
        // Without the computed head, under the Metal binding, the device takes it; under the CPU binding it does not.
        let mut plain = Shape::default();
        adds(&mut plain, &[atom("kind", false), atom("within", true)], &[Term::Var("x".into())], 4e6, false);
        assert!(plain.missing_none());
        let device = set_device(true);
        let p = place(&plain);
        drop(device);
        assert!(p.gpu && p.missing.is_empty(), "{p:?}");
        assert!(!place(&plain).gpu, "the CPU binding keeps it");
        // `@doc(x) = @doc(y)` and `@in(x, {…})` are row code, and so is `z = cat(x, y)` (a text the device makes);
        // `sha256(x)` is not, nor a cat or `lower(x)` in a recursive stratum (its rounds would join the text before it
        // is a symbol); `lower(x)` is (a text the device makes into its arena).
        let cmp = |l: Term, r: Term| Literal::Cmp { l, op: "=", r };
        let doc = |v: &str| Term::Call("doc".into(), [Term::Var(v.into())].into());
        let mut docs = Shape::default();
        adds(&mut docs, &[atom("kind", false), cmp(doc("x"), doc("y"))], &[], 1.0, false);
        assert!(docs.forms.contains(&Form::Comparison) && !docs.forms.contains(&Form::Computed));
        let made = || cmp(Term::Var("z".into()), Term::Call("cat".into(), [Term::Var("x".into()), Term::Var("y".into())].into()));
        let mut cat = Shape::default();
        adds(&mut cat, &[atom("kind", false), made()], &[], 1.0, false);
        assert!(cat.forms.contains(&Form::Comparison) && !cat.forms.contains(&Form::Computed), "{:?}", cat.forms);
        let mut rec = Shape::default();
        adds(&mut rec, &[atom("kind", false), made()], &[], 1.0, true);
        assert!(rec.forms.contains(&Form::Computed));
        let lower = || cmp(Term::Var("z".into()), Term::Call("lower".into(), [Term::Var("x".into())].into()));
        let mut len = Shape::default();
        adds(&mut len, &[atom("kind", false), cmp(Term::Var("z".into()), Term::Call("sha256".into(), [Term::Var("x".into())].into()))], &[], 1.0, false);
        assert!(len.forms.contains(&Form::Computed));
        let mut low = Shape::default();
        adds(&mut low, &[atom("kind", false), lower()], &[], 1.0, false);
        assert!(!low.forms.contains(&Form::Computed), "{:?}", low.forms);
        let mut low_rec = Shape::default();
        adds(&mut low_rec, &[atom("kind", false), lower()], &[], 1.0, true);
        assert!(low_rec.forms.contains(&Form::Computed));
        // A tiny stratum stays on the CPU: the submission costs more than the work, even under the Metal binding.
        let mut small = Shape::default();
        adds(&mut small, &[atom("kind", false)], &[], 10.0, false);
        let device = set_device(true);
        let p = place(&small);
        drop(device);
        assert!(!p.gpu_cheaper && !p.gpu);
        assert_eq!(p.why(), "CPU: cheaper");
        // Join steps priced on each form: a large equi-join takes the sort and probe, its second probe of the same
        // relation pays no sort; a range inside a document takes the tile; a small lookup stays on the CPU's index.
        let est = |rel: &str, input: f64, output: f64, range: bool| JoinEst { rel: rel.into(), input, relation: 1e6, output, walked: (input, output), keys: 1, range, cost: 1.0 };
        let mut joined = Shape::default();
        joined.joins = vec![est("a", 1e6, 1e6, false), est("a", 1e6, 1e6, false), est("b", 1e6, 1e6, true), est("c", 10.0, 10.0, false)];
        let p = place(&joined);
        let forms: Vec<JoinForm> = p.joins.iter().map(|c| c.form).collect();
        assert_eq!(forms, vec![JoinForm::Probe, JoinForm::Probe, JoinForm::Tile, JoinForm::Index], "{p:?}");
        assert!(p.joins[1].probe_s < p.joins[0].probe_s, "the second probe finds the relation sorted");
        // A lookup into a view that costs several of a derived relation's (decision 0312: `within` by `n`, 5.5) is
        // priced at that many, on its index and in the stratum's CPU; one at a derived relation's cost adds nothing.
        let (plain, view) = (est("a", 1e4, 1e5, false), JoinEst { cost: 5.5, ..est("within", 1e4, 1e5, false) });
        assert!((view.index_s() - 5.5 * plain.index_s()).abs() < 1e-12);
        let one = |j: JoinEst| place(&Shape { work: 1.1e5, rules: 1, steps: 1, joins: vec![j], ..Shape::default() });
        let (p0, p1) = (one(plain.clone()), one(view.clone()));
        assert!((p1.cpu_s - p0.cpu_s - 4.5 * plain.index_s()).abs() < 1e-9, "{p0:?} {p1:?}");
        assert_eq!(p0.cpu_s, place(&Shape { work: 1.1e5, rules: 1, steps: 1, ..Shape::default() }).cpu_s, "a cost of one is the units' price");
        assert!(p1.joins[0].index_s > p0.joins[0].index_s);
        // A copy of an input (one scan, every row out) is the GPU's (decision 0325 step 6): the device reads the view it
        // makes and writes the head in place, so the host pays its run, its stage and its view, and no row. It was
        // priced level on either side while each row came back (decision 0321's `kind`). A join whose rows the device
        // would make past the bound head, where the CPU's walk keeps one witness (decision 0229), is priced with every
        // row on the device.
        let device = set_device(true);
        let mut copy = Shape { work: 50_000.0, rows: 50_000.0, scanned: 50_000.0, rules: 1, steps: 1, ..Shape::default() };
        copy.reads = vec![("node".into(), 50_000.0, 2, true)];
        let p = place(&copy);
        assert!(p.gpu && p.gpu_s < p.cpu_s / 3.0, "a copy is the GPU's: {p:?}");
        let mut derived = copy.clone();
        derived.reads = vec![("kind".into(), 50_000.0, 2, false)];
        assert!(place(&derived).gpu_s < p.gpu_s, "a derived relation is read in place: no view made");
        // The bias (decision 0321): a stratum the GPU's price puts at 1.5 times the CPU's is the GPU's; at 2.5 times,
        // the CPU's.
        let joined = |work: f64| Shape { work, rules: 1, steps: 1, joins: vec![JoinEst { rel: "a".into(), input: 1e6, relation: 1e6, output: 1e6, walked: (1e6, 1e6), keys: 1, range: false, cost: 1.0 }], ..Shape::default() };
        let device_s = place(&joined(2e6)).gpu_s - CPU_PER_STRATUM;
        let at = |ratio: f64| place(&joined(device_s / ratio / CPU_PER_WORK));
        let wide = at(1.5);
        assert!(wide.gpu && wide.gpu_s > wide.cpu_s, "the GPU taken though priced above the CPU: {wide:?}");
        assert!(!at(2.5).gpu, "{:?}", at(2.5));
        let cut = |walked: f64| Shape { work: 2e6, rows: 1e3, rules: 1, steps: 2, joins: vec![JoinEst { rel: "node".into(), input: 1e3, relation: 1e6, output: 1e3, walked: (1e3, walked), keys: 1, range: false, cost: 1.0 }], ..Shape::default() };
        assert!(place(&cut(1e3)).gpu_s < place(&cut(1e7)).gpu_s, "every row walked costs the device more");
        drop(device);
        // A recursive probe planned for the delta it saw at open is priced over the rows its relation gains: its
        // seeds' rows (here 500, against the 18 it saw) and one round more, at most as many again.
        let mut rounds = Shape { work: 100.0, ..Shape::default() };
        rounds.seeded("q", 500.0);
        rounds.probe("q", 3600.0, 9.0, 18.0);
        rounds.close_rounds();
        assert_eq!(rounds.work, 100.0 + 200.0 * 500.0 * 1.5);
        // A probe that makes more rows than it reads adds one round, not a divergent sum; one with no seeds keeps
        // the delta it was priced at.
        let mut many = Shape::default();
        many.probe("r", 40.0, 80.0, 20.0);
        many.close_rounds();
        assert_eq!(many.work, 2.0 * 20.0 * 2.0);
        assert!(many.probes.is_empty());
        // Recursion and computed terms are forms too.
        let mut rec = Shape::default();
        adds(&mut rec, &[atom("edge", false)], &[Term::Call("add".into(), vec![].into())], 1.0, true);
        assert!(rec.forms.contains(&Form::Recursion) && rec.forms.contains(&Form::Computed));
    }
}

/// A measurement aid only: `RERAIL_DEVICE_JOINS=off` keeps every stratum's joins on the CPU under a device binding,
/// so a run can be paired against the same binding with them on. Placement still prices and reports as usual.
pub(crate) fn device_joins_off() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_DEVICE_JOINS").is_ok_and(|v| v == "off"))
}
