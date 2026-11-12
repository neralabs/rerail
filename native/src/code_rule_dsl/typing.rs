//! Typing (docs/final-shape/03-architecture.md §13):
//! - `file_local`: the programs' file-local strata;
//! - `cross_rounds`: bounded cross-file rounds;
//! - `rounds`: the spike's lattice wavefront, the reference;
//! - `terms`: interned type terms, instantiation, proven/possible;
//! - `declare`: `rerail.types/v1` declarations;
//! - `deps`: dependency summaries.
pub mod file_local;
pub mod cross_rounds;
pub mod declare;
pub mod deps;
pub mod rounds;
pub mod terms;
// cratify: `oracle_tests` is rerail-engine's (it mounts it)
// cratify: `phase_tests` is rerail-engine's (it mounts it: they parse rule files, whose language the engine registers)
