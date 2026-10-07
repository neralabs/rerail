//! PHP usedness from the flow (`scope::php`): a binding is used where some read reaches it, as
//! Psalm 6.19.1's unused-variable analysis reads it; a subscript assignment (`$a[] = 1`) and a
//! statement-level self-update (`$a++`, `$a += 1`, `$a = $a + 1`) make a binding whose use is
//! its value's, and reference the binding they change without using it. A global-scope binding
//! of a variable some function's `global` statement names may be read whenever that function
//! runs: unknown unless a plain read uses it. A read a callee may take by reference, a binding
//! that reaches a `goto` (its label may read it) or an `unset` (which ends its value's life
//! there, running a destructor; Psalm counts it a use), and every binding of a scope whose
//! variables a construct reads by a name the source does not spell (`get_defined_vars()`,
//! `compact` of a non-literal, `$$name`, `eval`, `include`, a heredoc tree-sitter-php misreads),
//! may be used unseen: unknown unless a plain read uses it. So may a binding of a reference
//! (`&$x`, a by-reference parameter or `use`, `global`, `static`, one a closure's by-reference
//! `use` captures): what it assigns may be read through another name.
//!
//! A variable's occurrences are every `$x` node that names it: its bindings' names, its reads and
//! the nodes that name it otherwise (a closure's by-reference `use`, `unset`). A variable is a
//! scope's name; an arrow function's variable it does not bind is its enclosing scope's, which it
//! copies.

use super::super::php::{BindingKey, Flow, Modify, Quiet};
use super::super::{Form, NodeRef, ScopeKind, ScopeTable};
use super::{Facts, Steps};
use crate::policy_dsl::scope::BuildError;
use foldhash::{HashMap, HashMapExt};
use std::collections::BTreeSet;

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
    let kinds = ArrayKinds {
        arrays: crate::policy_dsl::language::kind_ids(
            table.language,
            &["array_creation_expression"],
        ),
        subscripts: crate::policy_dsl::language::kind_ids(
            table.language,
            &["subscript_expression"],
        ),
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
            // A change (`$a[] = 1`, `$i++;`) references the value but does not use it: the
            // binding it makes carries its use back. Unless the value is an array, though, a
            // subscript base may be an object whose `offsetSet` the write calls: a use of it, so
            // not known unused.
            match read.modify {
                Some(Modify::Element) => {
                    let key = (scope, read.name.clone(), index);
                    if !holds_array(table, flow, &kinds, &key, &mut Vec::new()) {
                        maybe.push(key);
                    }
                    continue;
                }
                Some(Modify::Update) => continue,
                None => {}
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
    // What a `goto`'s label or an `unset` may use (`scope::php`'s `jumped` and `unset`).
    maybe.extend(flow.jumped.iter().chain(&flow.unset).cloned());
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
    // A function's `global $x` reads the global `$x` whenever it is called, which the flow does not
    // follow: the global scope's bindings of `$x` may be used (Psalm's `referenced_globals`).
    let globals: BTreeSet<&str> = table
        .scopes
        .iter()
        .flat_map(|scope| scope.bindings.iter())
        .filter(|(_, bindings)| {
            bindings
                .iter()
                .any(|binding| binding.site.form == Form::Global)
        })
        .map(|(name, _)| name.as_ref())
        .collect();
    for (scope, scope_facts) in table.scopes.iter().zip(facts.iter_mut()) {
        if scope.kind != ScopeKind::Module {
            continue;
        }
        for (name, facts) in scope_facts.iter_mut() {
            if !globals.contains(name.as_ref()) {
                continue;
            }
            for fact in facts {
                if fact.used == Some(false) {
                    fact.used = None;
                    fact.nested_use = None;
                }
            }
        }
    }
    // A read of what a subscript assignment or a self-update made reads the value it changed: its
    // use carries back along each chain of changes.
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

/// Whether binding `key` holds an array on every path: it assigns an array literal, or a
/// subscript assignment made it from bindings that do (`$a[] = 1` on an undefined `$a` makes
/// one), or it is a closure's by-value `use` copy of bindings that do (the clause reads the
/// enclosing variable where the closure is created). A self-update (`$a .= v`, `$a = $a + v`)
/// holds what its operator makes, which is not read here. `seen`: the bindings this question
/// already asks about, which a loop may revisit.
fn holds_array(
    table: &ScopeTable,
    flow: &Flow,
    kinds: &ArrayKinds,
    key: &BindingKey,
    seen: &mut Vec<BindingKey>,
) -> bool {
    if seen.contains(key) {
        return true;
    }
    seen.push(key.clone());
    let Some(binding) = table.scopes[key.0 as usize]
        .bindings
        .get(key.1.as_ref())
        .and_then(|bindings| bindings.get(key.2))
    else {
        return false;
    };
    // A by-value `use` variable is bound at the node the enclosing scope reads.
    if let Some(read) = binding
        .name
        .and_then(|name| flow.read(name.start as usize))
        .filter(|read| read.scope != key.0 && read.name == key.1 && !read.aliased)
    {
        return read.bindings.iter().all(|&(scope, index)| {
            holds_array(table, flow, kinds, &(scope, read.name.clone(), index), seen)
        });
    }
    let site = &binding.site;
    if site
        .node
        .is_some_and(|node| kinds.subscripts.contains(&node.kind))
    {
        return flow.modifies.get(key).is_none_or(|previous| {
            previous
                .iter()
                .all(|previous| holds_array(table, flow, kinds, previous, seen))
        });
    }
    site.value
        .is_some_and(|value| kinds.arrays.contains(&value.kind))
}

/// The kinds `holds_array` reads: array literals, and the subscript a subscript assignment binds
/// its base at.
struct ArrayKinds {
    arrays: BTreeSet<u16>,
    subscripts: BTreeSet<u16>,
}
