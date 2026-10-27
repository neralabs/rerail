//! Python producers from Shared-structural-execution (`query/plan/{python_structure.rs,
//! call_batch.rs, import_screen.rs}`, `matcher/python_facts.rs`), out of `#[cfg(test)]`.
pub(crate) mod call_batch;
pub(crate) mod import_screen;
pub(crate) mod python_facts;
pub(crate) mod python_structure;
