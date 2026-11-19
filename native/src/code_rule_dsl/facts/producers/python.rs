//! Python producers from Shared-structural-execution (`query/plan/{python_structure.rs,
//! call_batch.rs, import_screen.rs}`, `matcher/python_facts.rs`), out of `#[cfg(test)]`.
pub mod call_batch;
/// The `py.*` fact families (list step 25, decision 0029), from core facts.
pub mod families;
pub mod import_screen;
pub mod python_facts;
pub mod python_structure;
