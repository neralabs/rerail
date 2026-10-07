//! PHP usedness from the flow (`scope::php`): a binding is used where some read reaches it, as
//! Psalm 6.19.1's unused-variable analysis reads it; a subscript assignment (`$a[] = 1`) makes a
//! binding whose use is its value's, and references the binding it changes without using it. A
//! read a callee may take by reference, and every binding of a scope whose variables a construct
//! reads by a name the source does not spell (`get_defined_vars()`, `compact` of a non-literal,
//! `$$name`, `eval`, `include`), may be used unseen: unknown unless a plain read uses it. So may a
//! binding of a reference (`&$x`, a by-reference parameter or `use`, `global`, `static`, one a
//! closure's by-reference `use` captures): what it assigns may be read through another name.
//!
//! A variable's occurrences are every `$x` node that names it: its bindings' names, its reads and
//! the nodes that name it otherwise (a closure's by-reference `use`, `unset`). A variable is a
//! scope's name; an arrow function's variable it does not bind is its enclosing scope's, which it
//! copies.

use super::super::php::Quiet;
use super::super::{NodeRef, ScopeTable};
use super::{Facts, Steps};
use crate::policy_dsl::scope::BuildError;
use std::collections::HashMap;

/// Per scope, per variable, aligned with the scope's bindings of the variable.
type ScopeFacts = Vec<HashMap<Box<str>, Vec<Facts>>>;

/// Each variable's occurrences in document order, by its scope and name.
pub(super) type Occurrences = HashMap<(u32, Box<str>), Vec<NodeRef>>;

/// The occurrences of every variable of the table's file.
pub(super) fn occurrences(
    table: &ScopeTable,
    steps: &mut Steps<'_>,
) -> Result<Occurrences, BuildError> {
    let mut occurrences = Occurrences::new();
    let Some(flow) = table.php.as_deref() else {
        return Ok(occurrences);
    };
    let variable = crate::policy_dsl::language::kind_ids(table.language, &["variable_name"]);
    for (index, scope) in table.scopes.iter().enumerate() {
        for (name, bindings) in &scope.bindings {
            for binding in bindings {
                steps.take()?;
                if let Some(node) = binding.name {
                    occurrences
                        .entry((index as u32, name.clone()))
                        .or_default()
                        .push(node);
                }
            }
        }
    }
    let named = flow
        .reads()
        .filter(|read| {
            !read.superglobal && &*read.name != "$this" && variable.contains(&read.node.kind)
        })
        .map(|read| (read.scope, &read.name, read.node))
        .chain(
            flow.mentions()
                .map(|(scope, name, node)| (*scope, name, *node)),
        );
    for (scope, name, node) in named {
        steps.take()?;
        let scope = flow.variable_scope(table, scope, name);
        occurrences
            .entry((scope, name.clone()))
            .or_default()
            .push(node);
    }
    for nodes in occurrences.values_mut() {
        nodes.sort_by_key(|node| (node.start, node.end));
        nodes.dedup_by_key(|node| (node.start, node.end));
    }
    Ok(occurrences)
}

pub(super) fn build(table: &ScopeTable, steps: &mut Steps<'_>) -> Result<ScopeFacts, BuildError> {
    let mut facts: ScopeFacts = table
        .scopes
        .iter()
        .map(|scope| {
            scope
                .bindings
                .iter()
                .map(|(name, bindings)| {
                    let known = Facts {
                        used: Some(false),
                        nested_use: Some(false),
                        referenced: Some(false),
                        ..Facts::default()
                    };
                    (name.clone(), vec![known; bindings.len()])
                })
                .collect()
        })
        .collect();
    let Some(flow) = table.php.as_deref() else {
        return Ok(facts);
    };
    let mut maybe: Vec<(u32, Box<str>, usize)> = Vec::new();
    for read in flow.reads() {
        steps.take()?;
        for &(scope, index) in read.bindings.iter() {
            let Some(fact) = facts[scope as usize]
                .get_mut(read.name.as_ref())
                .and_then(|facts| facts.get_mut(index))
            else {
                continue;
            };
            fact.referenced = Some(true);
            // The base of `$a[] = 1` references the value but does not use it: it changes it,
            // and the binding it makes carries its use back.
            if read.modify {
                continue;
            }
            if read.quiet == Quiet::ByReference {
                maybe.push((scope, read.name.clone(), index));
                continue;
            }
            fact.reads += 1;
            fact.used = Some(true);
            if read.scope != scope {
                fact.nested += 1;
                fact.nested_use = Some(true);
            }
        }
    }
    for (scope, name, index) in maybe {
        if let Some(fact) = facts[scope as usize]
            .get_mut(name.as_ref())
            .and_then(|facts| facts.get_mut(index))
            && fact.used == Some(false)
        {
            fact.used = None;
            fact.nested_use = None;
        }
    }
    for (scope, name, index) in &flow.aliased {
        if let Some(fact) = facts[*scope as usize]
            .get_mut(name.as_ref())
            .and_then(|facts| facts.get_mut(*index))
            && fact.used == Some(false)
        {
            fact.used = None;
            fact.referenced = fact.referenced.filter(|referenced| *referenced);
        }
    }
    // A read of what a subscript assignment made reads the value it changed: its use carries
    // back along each chain of changes.
    let mut changed = true;
    while changed {
        changed = false;
        for ((scope, name, index), previous) in &flow.modifies {
            steps.take()?;
            let Some(used) = facts[*scope as usize]
                .get(name.as_ref())
                .and_then(|facts| facts.get(*index))
                .map(|fact| fact.used)
            else {
                continue;
            };
            for (scope, name, index) in previous {
                if let Some(fact) = facts[*scope as usize]
                    .get_mut(name.as_ref())
                    .and_then(|facts| facts.get_mut(*index))
                {
                    let carried = match (fact.used, used) {
                        (Some(false), Some(true) | None) | (None, Some(true)) => used,
                        _ => continue,
                    };
                    fact.used = carried;
                    fact.referenced = carried.or(fact.referenced);
                    changed = true;
                }
            }
        }
    }
    for &scope in &flow.reads_all {
        for scope_facts in facts[scope as usize].values_mut() {
            for fact in scope_facts {
                if fact.used == Some(false) {
                    fact.used = None;
                    fact.referenced = fact.referenced.filter(|referenced| *referenced);
                }
                if fact.nested_use == Some(false) {
                    fact.nested_use = None;
                }
            }
        }
    }
    Ok(facts)
}
