//! Where a stratum runs, CPU or GPU, from its shape and the planner's estimates (decision 0095 §8).
//!
//! Before a stratum opens, its seeds' and probes' plans give an estimate of its work: the planner's own unit, the
//! rows a join step reads. Each backend has a price for that work:
//!
//! - **CPU:** a floor per stratum (open, index building, close) and a cost per unit of work. Both were measured on
//!   the Python pack, cold, on 2026-11-09: 5,476 strata, 177.9 M units of work and 20.6 s of joins. Strata above
//!   10^5 units cost 92–109 ns a unit; the 3,372 strata under 10^3 units cost 0.13 ms each.
//! - **GPU:** a submission's fixed cost (submit, schedule, wake: decision 0039's phases) and a cost per unit. Both are
//!   **assumed** until the relational lab (0095, step 0) measures the kernels: no relational kernel is built yet.
//!
//! The stratum's **shape** is the relational forms its plans use: joins, negation, aggregates, comparisons,
//! computed terms, text tests, recursion, tree axes. The device runs a stratum only when it has every form
//! (`DEVICE_FORMS`), and only under the Metal binding. Otherwise the stratum runs on the CPU, and the profile says
//! where the GPU would have been cheaper and which forms it lacks: the work the device must take on next, by its cost.

use super::{Mode, Step};
use crate::code_rule_dsl::logic::front::{AggOp, Literal, Term};
use std::cell::Cell;
use std::collections::BTreeSet;

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

/// The forms the Metal backend runs. Empty: no relational kernel is built yet (0095's order of work adds them, and
/// each one added here moves the strata that need only these to the device).
const DEVICE_FORMS: &[Form] = &[];

/// CPU: seconds per unit of planned work, and per stratum (measured; see the module's comment).
const CPU_PER_WORK: f64 = 100e-9;
const CPU_PER_STRATUM: f64 = 0.13e-3;
/// GPU: seconds per unit of planned work, and per submission (assumed; see the module's comment).
const GPU_PER_WORK: f64 = 2e-9;
const GPU_PER_SUBMISSION: f64 = 0.15e-3;
/// The GPU is chosen only when it is this much cheaper: the estimates are rough.
const MARGIN: f64 = 2.0;

thread_local! {
    /// Whether this thread's run may use the device (the run's binding, `typing::file_local::Binding::Metal`).
    static DEVICE: Cell<bool> = const { Cell::new(false) };
}

/// Sets whether this thread's run may place strata on the device; the previous value, to restore.
pub(in crate::code_rule_dsl) fn set_device(on: bool) -> bool {
    DEVICE.with(|d| d.replace(on))
}

/// A stratum's shape: its forms, and its planned work.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct Shape {
    pub(in crate::code_rule_dsl) forms: BTreeSet<Form>,
    pub(in crate::code_rule_dsl) work: f64,
    pub(in crate::code_rule_dsl) rules: usize,
}

impl Shape {
    /// Adds one planned rule: its steps in order, its head's terms and its planned work.
    pub(in crate::code_rule_dsl) fn add(&mut self, order: &[Step], head: &[Term], work: f64, recursive: bool) {
        self.rules += 1;
        self.work += work.max(0.0);
        if recursive {
            self.forms.insert(Form::Recursion);
        }
        if head.iter().any(|t| matches!(t, Term::Call(..))) {
            self.forms.insert(Form::Computed);
        }
        for step in order {
            if matches!(step.mode, Some(Mode::Range | Mode::Climb | Mode::Descend)) {
                self.forms.insert(Form::TreeAxis);
            }
            match &step.lit {
                Literal::Atom(a) => {
                    self.forms.insert(if a.neg { Form::Negation } else { Form::Join });
                    if a.rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) {
                        self.forms.insert(Form::TextTest);
                    }
                    if a.args.iter().any(|t| matches!(t, Term::Call(..))) {
                        self.forms.insert(Form::Computed);
                    }
                }
                Literal::Cmp { l, r, .. } => {
                    let computed = [l, r].iter().any(|t| matches!(t, Term::Call(..)));
                    self.forms.insert(if computed { Form::Computed } else { Form::Comparison });
                }
                Literal::Agg { op, atom, .. } => {
                    self.forms.insert(if matches!(op, AggOp::Join { .. }) { Form::StringJoin } else { Form::Aggregate });
                    if atom.rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) {
                        self.forms.insert(Form::TextTest);
                    }
                }
                Literal::Nearest { .. } => {
                    self.forms.insert(Form::Nearest);
                }
                Literal::Ast { .. } => {
                    self.forms.insert(Form::Matcher);
                }
            }
        }
    }
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

/// Places a stratum of `shape`.
pub(in crate::code_rule_dsl) fn place(shape: &Shape) -> Placement {
    let cpu_s = CPU_PER_STRATUM + CPU_PER_WORK * shape.work;
    let gpu_s = GPU_PER_SUBMISSION + GPU_PER_WORK * shape.work;
    let missing: Vec<Form> = shape.forms.iter().copied().filter(|f| !DEVICE_FORMS.contains(f)).collect();
    let device = DEVICE.with(Cell::get);
    let gpu_cheaper = gpu_s * MARGIN < cpu_s;
    Placement { gpu: device && gpu_cheaper && missing.is_empty(), gpu_cheaper, missing, cpu_s, gpu_s, device }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::front::Atom;

    fn atom(rel: &str, neg: bool) -> Step {
        Step { at: 0, lit: Literal::Atom(Atom { rel: rel.to_owned(), args: vec![Term::Var("x".into())], neg, gated: false }), mode: None }
    }

    #[test]
    fn a_stratum_is_placed_by_its_price_and_named_by_the_forms_the_device_lacks() {
        // A large join with a negation and a text test: the GPU's price is lower, and every form is missing.
        let mut big = Shape::default();
        big.add(&[atom("kind", false), atom("strtest_3", false), atom("within", true)], &[Term::Var("x".into())], 4e6, false);
        assert_eq!(big.forms, [Form::Join, Form::Negation, Form::TextTest].into_iter().collect());
        let p = place(&big);
        assert!(p.gpu_cheaper && !p.gpu, "{p:?}");
        assert_eq!(p.missing, vec![Form::Join, Form::Negation, Form::TextTest]);
        assert!(p.why().contains("no join, no negation, no text test"), "{}", p.why());
        // A tiny stratum stays on the CPU: the submission costs more than the work.
        let mut small = Shape::default();
        small.add(&[atom("kind", false)], &[], 10.0, false);
        let p = place(&small);
        assert!(!p.gpu_cheaper && !p.gpu);
        assert_eq!(p.why(), "CPU: cheaper");
        // Recursion and computed terms are forms too.
        let mut rec = Shape::default();
        rec.add(&[atom("edge", false)], &[Term::Call("add".into(), vec![])], 1.0, true);
        assert!(rec.forms.contains(&Form::Recursion) && rec.forms.contains(&Form::Computed));
    }
}
