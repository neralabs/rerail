//! A program's `ast(d, n, { … })` atoms, run by the transducer's matchers (decision 0106).
//!
//! Each atom becomes an input relation, one per distinct matcher and captured names: `rr_ast_<k>(n)` without
//! captures, `rr_ast_<k>_<m>(n, x1, …, xm)` with `m` (decision 0107), and `docof(n, d)` where the rule reads `d`
//! elsewhere. Each chunk fills the relations by running the matchers' wave over its documents' tapes
//! (`facts::vtab::syntax::with_matchers`); a captured matcher's rows read W's bindings at each matched root. The
//! joins read the matches as any base relation.

pub use crate::code_rule_dsl::facts::grammar_shape::single_field;
#[cfg(any(test, feature = "test-hooks"))]
use crate::code_rule_dsl::facts::grammar_shape::{field_kinds, leaf_kind};

pub use crate::code_rule_dsl::facts::core::ast_matches::{MOST_CAPTURES, PREFIX, columns};

/// Each relation's matcher compiled for the transducer, over the program's first `.policy` (its utils), by the
/// registered compiler (`facts::vtab::syntax::AstCompiler`). A matcher that does not lower fails the run, naming it.
pub(in crate::code_rule_dsl) fn compiled(matchers: &[(String, String, Vec<String>)], policies: &[serde_json::Value]) -> Result<crate::code_rule_dsl::facts::vtab::syntax::Matchers, String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "ast matchers lowered".into());
    crate::code_rule_dsl::facts::vtab::syntax::compile_matchers(matchers, policies)
}

#[cfg(test)]
mod tests;
