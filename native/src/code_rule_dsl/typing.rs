//! Typing (docs/final-shape/03-architecture.md §13):
//! - `file_local`: the programs' file-local strata;
//! - `cross_rounds`: bounded cross-file rounds;
//! - `rounds`: the spike's lattice wavefront, the reference;
//! - `terms`: interned type terms, instantiation, proven/possible;
//! - `declare`: `rerail.types/v1` declarations;
//! - `deps`: dependency summaries.
pub(crate) mod file_local;
pub(crate) mod cross_rounds;
pub(crate) mod declare;
pub(crate) mod deps;
pub(crate) mod rounds;
pub(crate) mod terms;
#[cfg(test)]
mod oracle_tests;
