//! Specialized executors for shapes of rules (the design "Registering an executor for a join shape", 2026-11-26).
//!
//! A shape is a structure of a component's rules, recursive ones included, that has a better form as a whole than as
//! its joins run one at a time. An **executor** recognizes its shape on an opened component and gives a **form**, the
//! code that runs it. A component with a form runs it in place of its semi-naive waves, in two halves around the
//! device launches it wants (`prepare`, `finish`), so a level's launches still go to the device as one wave (decision
//! 0095 §5).
//!
//! The executors are this list, `EXECUTORS`, tried in order; the first whose shape the component has gives its form.
//! Each has a name: the profile's decisions and counts, the frames file (`RERAIL_JOIN_FRAMES`) and the switch that turns
//! it off (`RERAIL_SHAPE_<NAME>=off`, its name upper-cased with `_` for spaces; `RERAIL_SHAPES=off` turns all off) use
//! it. A form's rows are the rules' rows: the tests run each shape against the generic evaluation, its switch off.
use super::{Component, Store, closure};
use rerail_accel::{Kernel, Output};
use rerail_blocks::vector::Vector;

/// What a form did, for the run's report: nodes reached, rows made, and which way.
pub(super) struct Ran {
    pub(super) nodes: usize,
    pub(super) rows: usize,
    pub(super) how: String,
}

/// A specialized executor: recognizes its shape on an opened component.
pub(super) trait Executor: Sync {
    /// Its name, as the switch, the profile and the frames file spell it.
    fn name(&self) -> &'static str;
    /// The form for component `c`, or `None` where `c` does not have the shape: the component then runs generically.
    fn open(&self, c: &Component) -> Option<Box<dyn Form>>;
}

/// A recognized shape, run in place of the component's waves.
pub(super) trait Form: Send {
    /// The executor's name, and the form's own where one executor gives several (`closure`: "reach", "keyed walk").
    fn name(&self) -> &'static str;
    /// The host's work, and the device launches it wants. `None`: the form could not run (an edge search failed);
    /// the component then runs semi-naive from the rows it has.
    fn prepare(&mut self, store: &mut Store, syms: &mut super::Interner) -> Option<Vec<Kernel>>;
    /// The launches' outputs in (one a launch, in order); the rows out, into the component's relations.
    fn finish(&mut self, outs: Vec<Output>, store: &mut Store) -> Option<Ran>;
    /// The relation whose seeds the form takes itself (the design's S4: the start joined in the climb), never written
    /// to it: the seed loop collects their rows and hands them over (`take_seeds`) before `prepare`.
    fn seeds(&self) -> Option<usize> {
        None
    }
    /// The seeds' rows of relation `seeds()`, collected (`k` values a row).
    fn take_seeds(&mut self, _rows: Vector<i64>) {}
}

/// The executors, in the order they are tried.
pub(super) const EXECUTORS: &[&dyn Executor] = &[&Closure];

/// Tests: the executors' names, in the order they are tried (the oracle cases' table checks each has its cases).
#[cfg(any(test, feature = "test-hooks"))]
pub fn executor_names() -> Vec<&'static str> {
    EXECUTORS.iter().map(|e| e.name()).collect()
}

/// Whether executor `name` is turned off for this run (`RERAIL_SHAPES=off`, or `RERAIL_SHAPE_<NAME>=off`).
fn off(name: &str) -> bool {
    let var = format!("RERAIL_SHAPE_{}", name.to_ascii_uppercase().replace(' ', "_"));
    let all = rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_SHAPES").is_ok_and(|v| v == "off"));
    all || std::env::var(var).is_ok_and(|v| v == "off")
}

/// The form of component `c`: the first executor's that has its shape and is not turned off.
pub(super) fn open(c: &Component) -> Option<Box<dyn Form>> {
    EXECUTORS.iter().filter(|e| !off(e.name())).find_map(|e| e.open(c))
}

/// The closure forms (decision 0098 §1: a walk along an edge, as the edge's closure over distinct nodes; decision
/// 0570: a walk keyed by its row, as a worklist in one wave, with its ends answered in the climb, decision 0561).
struct Closure;

impl Executor for Closure {
    fn name(&self) -> &'static str {
        "closure"
    }
    fn open(&self, c: &Component) -> Option<Box<dyn Form>> {
        let cl = closure::recognize(c)?;
        Some(Box::new(ClosureForm { name: if cl.is_keyed() { "keyed walk" } else { "reach" }, cl, prep: None }))
    }
}

struct ClosureForm {
    name: &'static str,
    cl: closure::Closure,
    /// Between the halves: what `prepare` made for `finish`.
    prep: Option<closure::Prepared>,
}

impl Form for ClosureForm {
    fn name(&self) -> &'static str {
        self.name
    }
    fn prepare(&mut self, store: &mut Store, syms: &mut super::Interner) -> Option<Vec<Kernel>> {
        let mut p = closure::prepare(&self.cl, store, syms)?;
        let launches = std::mem::take(&mut p.launches);
        self.prep = Some(p);
        Some(launches)
    }
    fn finish(&mut self, outs: Vec<Output>, store: &mut Store) -> Option<Ran> {
        closure::finish(&self.cl, self.prep.take()?, outs, store)
    }
    fn seeds(&self) -> Option<usize> {
        self.cl.folds_seeds()
    }
    fn take_seeds(&mut self, rows: Vector<i64>) {
        self.cl.take_seeds(rows);
    }
}

/// Runs form `form`'s half `f` of component `component`, under frames naming the form (`RERAIL_JOIN_FRAMES`): a
/// profiler's samples in a specialized executor are then told from a generic join's (`framed`, `FormFrames`).
pub(super) fn framed<R>(form: &'static str, component: &str, f: &mut dyn FnMut() -> R) -> R {
    if !super::join_frames() {
        return f();
    }
    let h = { use std::hash::{Hash, Hasher}; let mut h = rustc_hash::FxHasher::default(); form.hash(&mut h); h.finish() };
    super::join_frame_line(format!("form-frame {h:016x} {form} {component}"));
    super::framed::<super::FormFrames, R>(h, f)
}

/// The component's rows, by who made them: `form`'s (none: "") rows made and rows walked (nodes reached and rows
/// made), and the rows the generic joins' steps made (each step's out bindings, as `step_counts`). Counted in the
/// run's profile, and written to the frames file (`RERAIL_JOIN_FRAMES`).
pub(super) fn account(form: &'static str, component: &str, rows: u64, visits: u64, generic: u64) {
    use crate::code_rule_dsl::logic::lanes::prof::count;
    if rows == 0 && visits == 0 && generic == 0 {
        return;
    }
    if !form.is_empty() {
        count(form, "rows a specialized executor made", rows);
        count(form, "rows a specialized executor walked", visits);
    }
    count("generic joins", "rows the generic joins' steps made", generic);
    if super::join_frames() {
        super::join_frame_line(format!("rows {} {component} made {rows} walked {visits} generic {generic}", if form.is_empty() { "-" } else { form }));
    }
}
