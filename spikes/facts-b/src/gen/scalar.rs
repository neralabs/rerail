//! Scalar and constant evaluation for the generator's per-record operators: the reference
//! evaluator's (`rerail_facts::reference`), so the dataflow and the reference share one
//! definition of SQLite's expression semantics and the gate's comparison tests the relational
//! and delta machinery.
use rerail_facts::ir::{Expr, Rel, RepoSource};
use rerail_facts::reference::{self, Evaluator};
use rerail_facts::sql::value::V;

pub fn eval<S>(e: &Expr<S>, row: &[V], outer: &[&[V]]) -> Result<V, String> {
    reference::eval_scalar(e, row, outer)
}

/// Rows of a relation that scans nothing (constants, `json_each` over constants), in order.
pub fn constant(r: &Rel<RepoSource>) -> Result<Vec<Vec<V>>, String> {
    let src = |_: &RepoSource| -> Result<Vec<Vec<V>>, String> { Err("constant relation scans a source".into()) };
    Evaluator::new(&src).rel(r, &[])
}
