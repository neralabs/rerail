//! What a program's evaluations cost, in steps (decision 0334): the prediction a task's budget and states are sized
//! from, and what each run measured, for the cost store (`logic::costs`).
//!
//! - **Steps** are the transducer's work units, the same on the CPU JIT and on Metal (each counts what it charges
//!   against the budget, `TaskStatus::reserved`) and on every machine. So a rate is a program's, not a machine's.
//! - **A task's size** is its subtree's nodes, and for a program that tests text, its root's bytes over eight: a
//!   text test's steps grow with the bytes it reads (Q19's clause).
//! - **The prediction** is the program's rate (steps per unit of size, from the store) times the task's size; a
//!   program with no rate yet has the prior: size × words × [`PRIOR_STEPS_PER_WORD`]. Never below [`MIN_PREDICTED`].
//! - **Observations** (an evaluation's steps, size, prediction, and whether it stopped over its budget) go to the
//!   cost store (`rerail_accel::program_steps`, written by `logic::costs`), which gives the rates back. A misprediction is an observation that
//!   stopped over its budget.

use super::Program;
pub use rerail_accel::program_steps::{Observation, Rate, drain, install, keep, keeping, observe, rate};

/// The prior's steps per node and word, for a program with no rate: a step is one word at one node, and a path's
/// words at a node run once or twice (a fork, a retry). The budget's slack covers the rest.
pub const PRIOR_STEPS_PER_WORD: u64 = 2;
/// The least prediction: a task's launch costs more than its first steps.
pub const MIN_PREDICTED: u64 = 256;
/// W's matcher's steps per node of a root's subtree, for a clause with no measured rate (decision 0334): a pattern
/// node's test at each node it visits, a few a node.
pub const W_PRIOR_STEPS_PER_NODE: u64 = 4;

/// The cost store's key for W's matcher at a clause's roots: the clause's root program's, marked as W's. W's steps
/// (its utility steps, `matcher::end_clause`) are counted as the transducer's: one test at one node.
pub fn w_key(program: &Program) -> String {
    format!("w:{}", program.plain_digest())
}

/// W's predicted steps to confirm roots whose subtrees hold `nodes` nodes in all: its rate, else the prior.
pub fn predict_w(program: &Program, nodes: u64) -> u64 {
    match rate(&w_key(program)) {
        Some(rate) if rate.samples > 0 && rate.steps_per_unit.is_finite() => (rate.steps_per_unit * nodes as f64).ceil() as u64,
        _ => nodes.saturating_mul(W_PRIOR_STEPS_PER_NODE),
    }
}

/// The program's micro walk's predicted steps from roots whose subtrees hold `nodes` nodes, with `slots` text tests
/// on a file of `bytes` bytes (decision 0334 §5, the prefilter's plan): the interpreter's prior without the text, since
/// the walk reads each text test as bits (the term index's row, else one pass of the text layer over the file, its
/// bytes over eight a slot), never a test per node. A walk has a bound of its own (`micro::steps`) and fails there.
pub fn predict_walk(program: &Program, nodes: u64, slots: usize, bytes: u64) -> u64 {
    nodes
        .saturating_mul(program.words.len().max(1) as u64)
        .saturating_mul(PRIOR_STEPS_PER_WORD)
        .saturating_add((slots as u64).saturating_mul(bytes / 8))
        .max(MIN_PREDICTED)
}

/// A resident label pass's predicted steps per node of a task's subtree (`assign_resident`): its rate, else the
/// prior.
pub fn per_node(program: &Program) -> u64 {
    rate(program.plain_digest()).map_or(program.words.len().max(1) as u64 * PRIOR_STEPS_PER_WORD, |rate| rate.steps_per_unit.ceil().max(1.0) as u64)
}

/// A task's size: its subtree's nodes, and its root's bytes over eight where the program tests text.
pub fn size(program: &Program, nodes: u32, bytes: u32) -> u64 {
    let reads_text = !program.predicates.is_empty() || !program.value_programs.is_empty();
    u64::from(nodes.max(1)) + if reads_text { u64::from(bytes) / 8 } else { 0 }
}

/// A task's predicted steps from its size: the program's rate, else the prior.
pub fn predict(program: &Program, rate: Option<Rate>, size: u64) -> u64 {
    let predicted = match rate {
        Some(rate) if rate.samples > 0 && rate.steps_per_unit.is_finite() => (rate.steps_per_unit * size as f64).ceil() as u64,
        _ => size.saturating_mul(program.words.len().max(1) as u64).saturating_mul(PRIOR_STEPS_PER_WORD),
    };
    predicted.max(MIN_PREDICTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_predicts_and_the_prior_stands_in_without_one() {
        let program = Program::compile_universe().unwrap();
        let words = program.words.len() as u64;
        assert_eq!(predict(&program, None, 100), (100 * words * PRIOR_STEPS_PER_WORD).max(MIN_PREDICTED));
        assert_eq!(predict(&program, Some(Rate { steps_per_unit: 50.0, samples: 3 }), 100), 5_000);
        assert_eq!(predict(&program, Some(Rate { steps_per_unit: 0.1, samples: 3 }), 100), MIN_PREDICTED);
    }

    /// A misprediction reprices its program at once, for the evaluations not yet started: no store, no next run.
    #[test]
    fn an_observation_reprices_its_program_at_once() {
        let program = "an_observation_reprices_its_program_at_once".to_owned();
        install([(program.clone(), Rate { steps_per_unit: 1e-6, samples: 1 })].into_iter().collect());
        observe([Observation { program: program.clone(), steps: 4_000, size: 99, predicted: 1, over_budget: true }]);
        let repriced = rate(&program).unwrap();
        assert_eq!(repriced.samples, 2);
        assert!((repriced.steps_per_unit - 40.0).abs() < 1e-3, "{repriced:?}");
        // A program never priced is priced from its first observation.
        let fresh = "an_observation_reprices_its_program_at_once/fresh".to_owned();
        observe([Observation { program: fresh.clone(), steps: 300, size: 100, predicted: 0, over_budget: false }]);
        assert_eq!(rate(&fresh), Some(Rate { steps_per_unit: 3.0, samples: 1 }));
    }
}
