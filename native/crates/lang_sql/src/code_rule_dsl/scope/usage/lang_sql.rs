//! SQL's usage index (moved from `usage.rs` by the crate split).

use super::*;

/// SQL's usage index (`Usage`), its crate's.
pub(crate) trait SqlUsage: Sized {
    /// SQL's index, from the file's query blocks (`sql::SqlScopes::usage`): a binding is used,
    /// and referenced, where AL05's walk finds a reference to it, a CTE also where a relation
    /// selects from it, and a DML target by its own statement. SQL exports nothing.
    fn sql(table: &ScopeTable, steps: &mut Steps<'_>) -> Result<Self, BuildError>;
}

impl SqlUsage for Usage {
    /// SQL's index, from the file's query blocks (`sql::SqlScopes::usage`): a binding is used,
    /// and referenced, where AL05's walk finds a reference to it, a CTE also where a relation
    /// selects from it, and a DML target by its own statement. SQL exports nothing.
    fn sql(table: &ScopeTable, steps: &mut Steps<'_>) -> Result<Self, BuildError> {
        let mut facts: Vec<HashMap<Box<str>, Vec<Facts>>> = table
            .scopes
            .iter()
            .map(|scope| {
                scope
                    .bindings
                    .iter()
                    .map(|(name, chain)| (name.clone(), vec![Facts::default(); chain.len()]))
                    .collect()
            })
            .collect();
        for (scope, name, index, uses) in table.sql.iter().flat_map(|sql| sql.usage()) {
            steps.take()?;
            if let Some(fact) = facts[scope as usize]
                .get_mut(name)
                .and_then(|chain| chain.get_mut(index))
            {
                fact.reads = uses.reads;
                fact.nested = uses.nested;
                fact.used = Some(uses.reads > 0);
                fact.referenced = Some(uses.reads > 0);
                fact.nested_use = Some(uses.nested > 0);
            }
        }
        let sites = name_sites(table, steps)?;
        Ok(Self {
            facts,
            sites,
            exports: Vec::new(),
            comment_exports: BTreeSet::new(),
            occurrences: HashMap::new(),
            privates: Vec::new(),
            private_names: Vec::new(),
            before_local: Vec::new(),
            cost: steps.cost,
        })
    }
}
