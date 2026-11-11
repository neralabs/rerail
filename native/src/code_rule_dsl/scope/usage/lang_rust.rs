//! Rust's usage index (moved from `usage.rs` by the crate split).

use super::*;

/// Rust's usage index (`Usage`), its crate's.
pub(crate) trait RustUsage: Sized {
    /// Rust's index: the reads rustc's unused lints count, with what macros may read unknown
    /// (`rust`).
    #[cfg_attr(not(test), allow(dead_code))]
    fn rust<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Self, BuildError>;
}

impl RustUsage for Usage {
    /// Rust's index: the reads rustc's unused lints count, with what macros may read unknown
    /// (`rust`).
    #[cfg_attr(not(test), allow(dead_code))]
    fn rust<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Self, BuildError> {
        let (facts, occurrences) = rust::build(table, root, steps)?;
        let sites = name_sites(table, steps)?;
        Ok(Self {
            facts,
            sites,
            exports: Vec::new(),
            comment_exports: BTreeSet::new(),
            occurrences,
            privates: Vec::new(),
            private_names: Vec::new(),
            before_local: Vec::new(),
            cost: steps.cost,
        })
    }
}
