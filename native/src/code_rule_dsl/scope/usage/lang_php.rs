//! PHP's usage index (moved from `usage.rs` by the crate split).

use super::*;

/// PHP's usage index (`Usage`), its crate's.
pub(crate) trait PhpUsage: Sized {
    /// PHP's index, from its flow (`php`).
    fn php(table: &ScopeTable, steps: &mut Steps<'_>) -> Result<Self, BuildError>;
}

impl PhpUsage for Usage {
    /// PHP's index, from its flow (`php`).
    fn php(table: &ScopeTable, steps: &mut Steps<'_>) -> Result<Self, BuildError> {
        let facts = php::build(table, steps)?;
        let occurrences = php::occurrences(table, steps)?;
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
