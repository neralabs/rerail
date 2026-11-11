//! Reconstructible contribution rows: retain a complete context key and project pure fields.
use super::{Decision, scalars::Expression};
#[derive(Clone, Debug, serde::Serialize)]
pub struct Projection {
    pub context_width: usize,
    pub heads: Vec<Expression>,
    pub decision: Decision,
}
pub fn projection(
    context_width: usize,
    heads: Vec<Expression>,
    predicates: Vec<Expression>,
) -> Projection {
    let reason = if heads.is_empty() {
        Some("No physical head columns")
    } else if heads.iter().any(|e| {
        !matches!(e,Expression::Column(c) if *c<context_width)
            && !matches!(e, Expression::Integer(_))
    }) {
        Some("Computed or opaque head values require retained evaluation results")
    } else if predicates.iter().any(|e| !e.stable()) {
        Some("Effectful or checked predicates require retained evaluation results")
    } else {
        None
    };
    Projection { context_width, heads, decision: Decision::new(reason.is_none(),reason.unwrap_or(
        "Complete context keys and pure direct head projections reconstruct contribution rows without duplicate storage")) }
}
