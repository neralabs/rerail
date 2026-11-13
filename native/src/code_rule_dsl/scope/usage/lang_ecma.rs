//! TypeScript's and JavaScript's usage index (moved from `usage.rs` by the crate split).

use super::*;

/// TypeScript's and JavaScript's usage index (`Usage`), its crate's.
pub(crate) trait EcmaUsage: Sized {
    /// TypeScript's and JavaScript's index: ESLint's no-unused-vars (`ecma`).
    fn ecma<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Self, BuildError>;
}

impl EcmaUsage for Usage {
    /// TypeScript's and JavaScript's index: ESLint's no-unused-vars (`ecma`).
    fn ecma<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Self, BuildError> {
        let built = ecma::build(table, root, steps)?;
        let sites = name_sites(table, steps)?;
        let mut private_names = built
            .privates
            .iter()
            .enumerate()
            .flat_map(|(member, private)| {
                private
                    .declarations
                    .iter()
                    .enumerate()
                    .map(move |(index, declaration)| {
                        (declaration.name, member as u32, index as u32)
                    })
            })
            .collect::<Vec<_>>();
        private_names.sort_by_key(|(name, _, _)| (name.start, name.end));
        Ok(Self {
            facts: built.facts,
            sites,
            exports: Vec::new(),
            comment_exports: built.comment_exports,
            occurrences: built.occurrences,
            privates: built.privates,
            private_names,
            before_local: Vec::new(),
            cost: steps.cost,
        })
    }
}
