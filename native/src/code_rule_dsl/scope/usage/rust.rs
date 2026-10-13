//! Rust's usage index: which references read each binding, as rustc's unused lints
//! (`unused_variables`, `unused_imports`, `dead_code`) count them in one file. A name a macro token
//! tree mentions (`println!("{}", x)`, a format string's `{x}`) may be read once the macro expands:
//! a binding such a mention may reach, and that nothing else reads, is not known to be unused.

use super::super::rust::{MACRO, resolve_index};
use super::super::{Access, Import, NodeRef, Reference, ScopeKind, ScopeTable, TYPE, VALUE};
use super::{Facts, Steps};
use crate::code_rule_dsl::scope::BuildError;
use ast_grep_core::{Doc, Node};
use foldhash::HashMap;

/// The facts of every binding of `table` (per scope, per name, aligned with the chains) and each
/// variable's definition names and references, in document order.
#[allow(clippy::type_complexity)]
pub(super) fn build<D: Doc>(
    table: &ScopeTable,
    root: &Node<'_, D>,
    steps: &mut Steps<'_>,
) -> Result<
    (
        Vec<HashMap<Box<str>, Vec<Facts>>>,
        HashMap<(u32, Box<str>), Vec<NodeRef>>,
    ),
    BuildError,
> {
    let text = root.text();
    let base = root.range().start;
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
    let mut occurrences: HashMap<(u32, Box<str>), Vec<NodeRef>> = HashMap::default();
    for (scope, bindings) in table.scopes.iter().enumerate() {
        for (name, chain) in &bindings.bindings {
            for binding in chain {
                if let Some(node) = binding.name {
                    occurrences
                        .entry((scope as u32, name.clone()))
                        .or_default()
                        .push(node);
                }
            }
        }
    }
    let mut referenced = vec![HashMap::<Box<str>, Vec<bool>>::default(); table.scopes.len()];
    for reference in &table.references {
        steps.take()?;
        let Some(name) = text.get(reference.start - base..reference.end - base) else {
            continue;
        };
        let Some((scope, index)) = resolve_index(table, reference, name) else {
            continue;
        };
        let Some(fact) = facts[scope as usize]
            .get_mut(name)
            .and_then(|chain| chain.get_mut(index))
        else {
            continue;
        };
        fact.references += 1;
        referenced[scope as usize]
            .entry(name.into())
            .or_insert_with(|| vec![false; table.scopes[scope as usize].bindings[name].len()])
            [index] = true;
        if !reference.write {
            fact.reads += 1;
            if function_of(table, reference.scope) != function_of(table, scope) {
                fact.nested += 1;
            }
        }
        if let Some(node) = node_ref(root, reference.start, reference.end) {
            occurrences
                .entry((scope, name.into()))
                .or_default()
                .push(node);
        }
    }
    // `use super::x`, `use self::m::y` and `use crate::z` read the module binding they start
    // from: the item `x` of the parent module, `m` of this one, `z` of the crate root.
    for (scope, bindings) in table.scopes.iter().enumerate() {
        for chain in bindings.bindings.values() {
            for binding in chain {
                let Some(Import::Qualified(path)) = &binding.import else {
                    continue;
                };
                let mut segments = path.split("::");
                let Some(target) = (match segments.next() {
                    Some("self") => Some(module_of(table, scope as u32)),
                    Some("super") => {
                        let module = module_of(table, scope as u32);
                        table.scopes[module as usize]
                            .parent
                            .map(|parent| module_of(table, parent))
                    }
                    Some("crate") => Some(0),
                    _ => None,
                }) else {
                    continue;
                };
                let Some(first) = segments.next() else {
                    continue;
                };
                if let Some(fact) = facts[target as usize]
                    .get_mut(first)
                    .and_then(|chain| chain.first_mut())
                {
                    fact.reads += 1;
                    fact.references += 1;
                    if target != module_of(table, scope as u32) {
                        fact.nested += 1;
                    }
                    referenced[target as usize]
                        .entry(first.into())
                        .or_insert_with(|| {
                            vec![false; table.scopes[target as usize].bindings[first].len()]
                        })[0] = true;
                }
            }
        }
    }
    // Bindings a macro token tree may read.
    let mut maybe = vec![HashMap::<Box<str>, Vec<bool>>::default(); table.scopes.len()];
    for (start, end) in mentions(root) {
        steps.take()?;
        let Some(name) = text.get(start - base..end - base) else {
            continue;
        };
        let scope = innermost(table, start);
        let probe = Reference {
            start,
            end,
            scope,
            spaces: VALUE | TYPE | MACRO,
            at: start,
            deferred: false,
            forward: false,
            type_scope: false,
            write: false,
            mutates: false,
            access: Access::Load,
        };
        if let Some((scope, index)) = resolve_index(table, &probe, name) {
            maybe[scope as usize].entry(name.into()).or_insert_with(|| {
                vec![false; table.scopes[scope as usize].bindings[name].len()]
            })[index] = true;
        }
    }
    for (scope, names) in facts.iter_mut().enumerate() {
        for (name, chain) in names.iter_mut() {
            for (index, fact) in chain.iter_mut().enumerate() {
                let unseen = maybe[scope].get(name).is_some_and(|flags| flags[index]);
                let was_referenced = referenced[scope]
                    .get(name)
                    .is_some_and(|flags| flags[index]);
                fact.used = known(fact.reads > 0, unseen);
                fact.nested_use = known(fact.nested > 0, unseen);
                fact.referenced = known(was_referenced, unseen);
            }
        }
    }
    for list in occurrences.values_mut() {
        list.sort_by_key(|node| (node.start, node.end));
        list.dedup_by_key(|node| (node.start, node.end));
    }
    Ok((facts, occurrences))
}

/// `true` is known; `false` is unknown where a macro may read the name unseen.
fn known(value: bool, unseen: bool) -> Option<bool> {
    (value || !unseen).then_some(value)
}

/// The function or closure scope holding `scope`, or the module's (0) at the top level.
fn function_of(table: &ScopeTable, scope: u32) -> u32 {
    let mut current = Some(scope);
    while let Some(index) = current {
        let scope = &table.scopes[index as usize];
        if matches!(scope.kind, ScopeKind::Function | ScopeKind::Lambda) {
            return index;
        }
        current = scope.parent;
    }
    0
}

/// The module scope holding `scope` (the file's is 0).
fn module_of(table: &ScopeTable, scope: u32) -> u32 {
    let mut current = Some(scope);
    while let Some(index) = current {
        let scope = &table.scopes[index as usize];
        if scope.kind == ScopeKind::Module {
            return index;
        }
        current = scope.parent;
    }
    0
}

/// The innermost scope whose range holds `at`.
fn innermost(table: &ScopeTable, at: usize) -> u32 {
    let mut best = 0;
    for (index, scope) in table.scopes.iter().enumerate() {
        let best_range = &table.scopes[best].range;
        if scope.range.contains(&at) && scope.range.len() <= best_range.len() {
            best = index;
        }
    }
    best as u32
}

/// The node spanning exactly `start..end` under `root`.
fn node_ref<D: Doc>(root: &Node<'_, D>, start: usize, end: usize) -> Option<NodeRef> {
    root.dfs()
        .filter(|node| node.range() == (start..end))
        .last()
        .map(|node| NodeRef::of(&node))
}

/// The byte ranges of the names macro token trees mention: their identifier tokens, and the
/// `{name}` arguments of string literals in them (format strings' implicit captures).
fn mentions<D: Doc>(root: &Node<'_, D>) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    for tree in root.dfs().filter(|node| node.kind() == "token_tree") {
        // Nested token trees are visited on their own.
        for token in tree.children() {
            match token.kind().as_ref() {
                "identifier" => {
                    let range = token.range();
                    found.push((range.start, range.end));
                }
                "string_literal" | "raw_string_literal" => {
                    let start = token.range().start;
                    found.extend(
                        captures(&token.text()).map(|(from, to)| (start + from, start + to)),
                    );
                }
                _ => {}
            }
        }
    }
    found
}

/// The byte ranges of the identifiers that open a `{..}` field of a format string (`{x}`,
/// `{x:?}`), escaped `{{` aside.
fn captures(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let bytes = text.as_bytes();
    let mut at = 0;
    std::iter::from_fn(move || {
        while at < bytes.len() {
            if bytes[at] == b'{' {
                if bytes.get(at + 1) == Some(&b'{') {
                    at += 2;
                    continue;
                }
                let start = at + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end] == b'_' || bytes[end].is_ascii_alphanumeric())
                {
                    end += 1;
                }
                at = end.max(start);
                let first = bytes.get(start).copied().unwrap_or(b'0');
                if end > start && !first.is_ascii_digit() {
                    return Some((start, end));
                }
            } else {
                at += 1;
            }
        }
        None
    })
}

#[cfg(test)]
mod tests;
