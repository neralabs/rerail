use super::AstMatcher;
use super::binds::{BindingFilter, capture_range};
use super::facts::{Facts, Target, slot};
use super::resolve::{file_table, file_usage, table_reading, unparenthesized};
#[cfg(test)]
use super::spend_util_step;
#[cfg(test)]
use super::util::UNKNOWN;
use crate::policy_dsl::language::{Family, GrammarDialect};
use crate::policy_dsl::scope::Lookup;
use crate::policy_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::policy_dsl::scope::php::Defined;
use crate::policy_dsl::scope::{Reference, ScopeTable};
use crate::policy_dsl::value::NodeRef;
#[cfg(test)]
use ast_grep_core::matcher::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
#[cfg(test)]
use std::borrow::Cow;
#[cfg(test)]
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;

/// `resolves.binding` (a Rerail extension): the node is a reference to a file-local binding, and
/// each listed sub-rule matches that binding's node: its site, its value, its annotation. The
/// sub-rules share the caller's environment, as `has` does, so they may bind captures.
#[derive(Clone)]
pub(crate) struct BindingTest {
    pub(crate) language: GrammarDialect,
    /// How the binding is read: through `global` or not, and whether it must be the first.
    pub(crate) lookup: Lookup,
    /// TypeScript: whether a destructured name's literal element is its value (the default);
    /// `destructuredValue: false` gives it none, as `ReferenceTracker` and `getStaticValue` read.
    pub(crate) destructured_value: bool,
    /// `resolves.write: "last"`: the node is where ESLint reports its variable (the last write
    /// from the variable's own function scope, else the first definition's name), a reference or
    /// a binding's name node.
    pub(crate) write_last: bool,
    /// Kinds a binding's name node can have, for `write_last`.
    pub(crate) name_kinds: BTreeSet<u16>,
    /// Kinds a reference can have.
    pub(crate) reference_kinds: BTreeSet<u16>,
    /// Parenthesized expressions, which are transparent.
    pub(crate) paren_kinds: BTreeSet<u16>,
    pub(crate) options: Option<Arc<LanguageOptions>>,
    pub(crate) python: Option<Arc<PythonOptions>>,
    /// Filters over the binding, as `binds` has.
    pub(crate) filter: BindingFilter,
    /// Matches the construct that binds the name (`Site::node`).
    pub(crate) site: Option<AstMatcher>,
    /// Matches the value the binding carries (`ScopeTable::binding`).
    pub(crate) value: Option<AstMatcher>,
    /// Matches the binding's annotation.
    pub(crate) annotation: Option<AstMatcher>,
    /// TypeScript: matches some definition name or reference of the binding's variable.
    pub(crate) occurrence: Option<AstMatcher>,
}

#[cfg(test)]
thread_local! {
    /// The binding nodes whose sub-rules are being matched, with the sub-rule matching each: a
    /// chain that comes back to one (`var a = b, b = a`, `var x = x`) binds nothing and fails.
    static FOLLOWING: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
}

impl BindingTest {
    #[cfg(test)]
    pub(super) fn match_node<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        let reference = unparenthesized(&self.paren_kinds, node.clone());
        if self.write_last {
            return self.match_anchor(node, reference, env);
        }
        if !self.reference_kinds.contains(&reference.kind_id()) {
            return None;
        }
        let table = file_table(
            &reference,
            self.language,
            table_reading(self.options.as_deref(), self.python.as_deref()),
        )?;
        let found = table.reference(&reference.range())?;
        let name = reference.text();
        if php_undecided(&table, found) {
            UNKNOWN.set(true);
            return None;
        }
        let (binding, value) = table.binding_by(found, &name, self.lookup)?;
        let (scope, index) = resolved_index(&table, found, &name, self.lookup)?;
        let id = crate::policy_dsl::scope::usage::BindingId {
            scope,
            name: &name,
            index,
        };
        let usage = if self.filter.reads_usage() || self.occurrence.is_some() {
            Some(file_usage(&reference, &table)?)
        } else {
            None
        };
        let region = |capture: &str| capture_range(env, capture);
        let verdict = self.filter.verdict(&table, usage.as_deref(), id, &region);
        match verdict {
            Some(true) => {}
            Some(false) => return None,
            None => {
                UNKNOWN.set(true);
                return None;
            }
        }
        let targets = [
            (&self.site, binding.site.node),
            (&self.value, self.value_of(binding.site.node, value)),
            (&self.annotation, binding.site.annotation),
        ];
        let mut trial = Cow::Borrowed(env.as_ref());
        match_binding_nodes(&reference, targets, &mut trial)?;
        if let (Some(rule), Some(usage)) = (&self.occurrence, &usage) {
            match_occurrence(&reference, rule, usage.occurrences(id), &mut trial)?;
        }
        if let Cow::Owned(bound) = trial {
            *env = Cow::Owned(bound);
        }
        Some(node)
    }

    /// `resolves.write: "last"`: `reference` (the node without parentheses) is its variable's
    /// anchor, and the variable passes the filters and sub-rules.
    #[cfg(test)]
    fn match_anchor<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        reference: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        let kind = reference.kind_id();
        if !self.reference_kinds.contains(&kind) && !self.name_kinds.contains(&kind) {
            return None;
        }
        let table = file_table(
            &reference,
            self.language,
            table_reading(self.options.as_deref(), self.python.as_deref()),
        )?;
        let usage = file_usage(&reference, &table)?;
        let range = reference.range();
        let name = reference.text();
        let id = match table.reference(&range) {
            Some(found) => {
                let (scope, index) = table.resolve_lexical_index(found, &name)?;
                crate::policy_dsl::scope::usage::BindingId {
                    scope,
                    name: &name,
                    index,
                }
            }
            None => usage.named_at(range.start, range.end, kind).next()?,
        };
        let anchor = (range.start as u32, range.end as u32);
        if usage.facts(id).anchor != Some(anchor)
            || (self.lookup.only && table.chain(id).len() != 1)
        {
            return None;
        }
        let region = |capture: &str| capture_range(env, capture);
        match self.filter.verdict(&table, Some(&usage), id, &region) {
            Some(true) => {}
            Some(false) => return None,
            None => {
                UNKNOWN.set(true);
                return None;
            }
        }
        let binding = table.binding_of(id);
        let definitions = table.chain(id).len();
        let value = binding
            .site
            .value
            .filter(|_| definitions == 1 && binding.writes == 0);
        let targets = [
            (&self.site, binding.site.node),
            (&self.value, self.value_of(binding.site.node, value)),
            (&self.annotation, binding.site.annotation),
        ];
        let mut trial = Cow::Borrowed(env.as_ref());
        match_binding_nodes(&reference, targets, &mut trial)?;
        if let Some(rule) = &self.occurrence {
            match_occurrence(&reference, rule, usage.occurrences(id), &mut trial)?;
        }
        if let Cow::Owned(bound) = trial {
            *env = Cow::Owned(bound);
        }
        Some(node)
    }

    /// The binding's value, or none for a destructured name under `destructuredValue: false`. A
    /// destructured name's value is an element of its declarator's initializer, so it ends before
    /// the declarator does; a plain declarator's value ends with it.
    fn value_of(&self, site: Option<NodeRef>, value: Option<NodeRef>) -> Option<NodeRef> {
        value.filter(|value| {
            self.destructured_value || site.is_none_or(|site| site.end == value.end)
        })
    }

    /// The sub-rules, which may bind captures.
    pub(crate) fn sub_rules(&self) -> impl Iterator<Item = &AstMatcher> {
        [&self.site, &self.value, &self.annotation, &self.occurrence]
            .into_iter()
            .flatten()
    }
}

/// The binding a reference resolves to, by its scope and index among that scope's bindings of the
/// name: Python's lookup, TypeScript's lexical one, PHP's flow (one binding on every path).
fn resolved_index(
    table: &ScopeTable,
    reference: &Reference,
    name: &str,
    lookup: Lookup,
) -> Option<(u32, usize)> {
    match Family::of(table.language) {
        Family::Python if lookup == Lookup::default() => {
            table.resolve_python_index(reference, name)
        }
        Family::Python => table.python_index_by(reference, name, lookup),
        Family::Php | Family::Blade => table.php_index(reference),
        Family::Ecma => table.resolve_lexical_index(reference, name),
    }
}

/// PHP: whether a read's binding is not known, though some binding may reach it: several may, a
/// path may reach none, or PHP's dynamic symbol table may decide. A read no binding reaches is
/// decided (it resolves to none).
fn php_undecided(table: &ScopeTable, reference: &Reference) -> bool {
    Family::of(table.language).reads_php()
        && table.php_index(reference).is_none()
        && table
            .php
            .as_deref()
            .and_then(|flow| flow.read(reference.start))
            .is_some_and(|read| !read.bindings.is_empty() || read.defined == Defined::Unknown)
}

/// The query engine's view of `resolves.binding` (`facts::slot`): the gate, then the binding's
/// nodes for the listed sub-rules and its occurrences, which the engine matches them on.
impl BindingTest {
    pub(super) fn facts<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &MetaVarEnv<'tree, D>,
        facts: &mut Facts<'tree, D>,
    ) {
        let reference = unparenthesized(&self.paren_kinds, node.clone());
        if self.write_last {
            return self.anchor_facts(reference, env, facts);
        }
        if !self.reference_kinds.contains(&reference.kind_id()) {
            return;
        }
        let Some(table) = file_table(
            &reference,
            self.language,
            table_reading(self.options.as_deref(), self.python.as_deref()),
        ) else {
            return;
        };
        let Some(found) = table.reference(&reference.range()) else {
            return;
        };
        let name = reference.text();
        if php_undecided(&table, found) {
            facts.gate(-1, Target::None);
            return;
        }
        let Some((binding, value)) = table.binding_by(found, &name, self.lookup) else {
            return;
        };
        let Some((scope, index)) = resolved_index(&table, found, &name, self.lookup) else {
            return;
        };
        let id = crate::policy_dsl::scope::usage::BindingId {
            scope,
            name: &name,
            index,
        };
        let usage = if self.filter.reads_usage() || self.occurrence.is_some() {
            let Some(usage) = file_usage(&reference, &table) else {
                return;
            };
            Some(usage)
        } else {
            None
        };
        let region = |capture: &str| capture_range(env, capture);
        match self.filter.verdict(&table, usage.as_deref(), id, &region) {
            Some(true) => {}
            Some(false) => return,
            None => {
                facts.gate(-1, Target::None);
                return;
            }
        }
        facts.gate(1, Target::None);
        binding_node_facts(
            &reference,
            [
                (&self.site, slot::SITE, binding.site.node),
                (
                    &self.value,
                    slot::VALUE,
                    self.value_of(binding.site.node, value),
                ),
                (&self.annotation, slot::ANNOTATION, binding.site.annotation),
            ],
            0,
            facts,
        );
        if let (Some(_), Some(usage)) = (&self.occurrence, &usage) {
            occurrence_facts(&reference, usage.occurrences(id), 0, facts);
        }
    }

    /// `resolves.write: "last"` (see `match_anchor`).
    fn anchor_facts<'tree, D: Doc>(
        &self,
        reference: Node<'tree, D>,
        env: &MetaVarEnv<'tree, D>,
        facts: &mut Facts<'tree, D>,
    ) {
        let kind = reference.kind_id();
        if !self.reference_kinds.contains(&kind) && !self.name_kinds.contains(&kind) {
            return;
        }
        let Some(table) = file_table(
            &reference,
            self.language,
            table_reading(self.options.as_deref(), self.python.as_deref()),
        ) else {
            return;
        };
        let Some(usage) = file_usage(&reference, &table) else {
            return;
        };
        let range = reference.range();
        let name = reference.text();
        let id = match table.reference(&range) {
            Some(found) => {
                let Some((scope, index)) = table.resolve_lexical_index(found, &name) else {
                    return;
                };
                crate::policy_dsl::scope::usage::BindingId {
                    scope,
                    name: &name,
                    index,
                }
            }
            None => match usage.named_at(range.start, range.end, kind).next() {
                Some(id) => id,
                None => return,
            },
        };
        let anchor = (range.start as u32, range.end as u32);
        if usage.facts(id).anchor != Some(anchor)
            || (self.lookup.only && table.chain(id).len() != 1)
        {
            return;
        }
        let region = |capture: &str| capture_range(env, capture);
        match self.filter.verdict(&table, Some(&usage), id, &region) {
            Some(true) => {}
            Some(false) => return,
            None => {
                facts.gate(-1, Target::None);
                return;
            }
        }
        facts.gate(1, Target::None);
        let binding = table.binding_of(id);
        let definitions = table.chain(id).len();
        let value = binding
            .site
            .value
            .filter(|_| definitions == 1 && binding.writes == 0);
        binding_node_facts(
            &reference,
            [
                (&self.site, slot::SITE, binding.site.node),
                (
                    &self.value,
                    slot::VALUE,
                    self.value_of(binding.site.node, value),
                ),
                (&self.annotation, slot::ANNOTATION, binding.site.annotation),
            ],
            0,
            facts,
        );
        if self.occurrence.is_some() {
            occurrence_facts(&reference, usage.occurrences(id), 0, facts);
        }
    }
}

/// One row per listed sub-rule of a binding's nodes (`slot::SITE`, `VALUE`, `ANNOTATION` or their
/// redefines slots), in `anchor`'s file: flag 0 where the binding has no such node, else the node
/// (none where the tree has no node of that range and kind).
pub(super) fn binding_node_facts<'tree, D: Doc>(
    anchor: &Node<'tree, D>,
    targets: [(&Option<AstMatcher>, u16, Option<NodeRef>); 3],
    key: i64,
    facts: &mut Facts<'tree, D>,
) {
    let root = anchor.root().root();
    for (rule, slot, target) in targets {
        if rule.is_none() {
            continue;
        }
        match target {
            None => facts.push(slot, key, 0, Target::None),
            Some(target) => facts.push(
                slot,
                key,
                1,
                target.find(root.clone()).map_or(Target::None, Target::Node),
            ),
        };
    }
}

/// One row per occurrence (`slot::OCCURRENCE`), in order: the node, or none where the tree has no
/// node of that range and kind.
pub(super) fn occurrence_facts<'tree, D: Doc>(
    anchor: &Node<'tree, D>,
    occurrences: &[NodeRef],
    key: i64,
    facts: &mut Facts<'tree, D>,
) {
    let root = anchor.root().root();
    for occurrence in occurrences {
        facts.push(
            slot::OCCURRENCE,
            key,
            1,
            occurrence
                .find(root.clone())
                .map_or(Target::None, Target::Node),
        );
    }
}

/// Matches `rule` at the first of `occurrences` (nodes of `anchor`'s file) where it matches,
/// committing what it binds; `None` when it matches at none.
#[cfg(test)]
pub(super) fn match_occurrence<'tree, D: Doc>(
    anchor: &Node<'tree, D>,
    rule: &AstMatcher,
    occurrences: &[NodeRef],
    env: &mut Cow<MetaVarEnv<'tree, D>>,
) -> Option<()> {
    let root = anchor.root().root();
    for occurrence in occurrences {
        if !spend_util_step() {
            return None;
        }
        let Some(target) = occurrence.find(root.clone()) else {
            continue;
        };
        let mut trial = Cow::Borrowed(env.as_ref());
        if rule.match_node_with_env(target, &mut trial).is_some() {
            if super::each::active() && super::each::excluded(&trial) {
                continue;
            }
            if let Cow::Owned(bound) = trial {
                *env = Cow::Owned(bound);
            }
            return Some(());
        }
    }
    None
}

/// Matches each listed sub-rule against its binding node in `anchor`'s file, all in one trial
/// environment committed only when every one matches. A sub-rule whose node the binding lacks does
/// not match; a chain that comes back to a node its sub-rule is matching fails there.
#[cfg(test)]
pub(super) fn match_binding_nodes<'tree, D: Doc>(
    anchor: &Node<'tree, D>,
    targets: [(&Option<AstMatcher>, Option<NodeRef>); 3],
    env: &mut Cow<MetaVarEnv<'tree, D>>,
) -> Option<()> {
    let mut resolved = Vec::with_capacity(3);
    for (rule, target) in targets {
        if let Some(rule) = rule {
            resolved.push((rule, target?));
        }
    }
    let root = anchor.root().root();
    let mut trial = Cow::Borrowed(env.as_ref());
    for (rule, target) in resolved {
        let target = target.find(root.clone())?;
        if !spend_util_step() {
            return None;
        }
        let key = (target.node_id(), rule as *const AstMatcher as usize);
        if FOLLOWING.with_borrow(|following| following.contains(&key)) {
            return None;
        }
        FOLLOWING.with_borrow_mut(|following| following.push(key));
        let matched = rule.match_node_with_env(target, &mut trial);
        FOLLOWING.with_borrow_mut(|following| following.pop());
        matched?;
    }
    if let Cow::Owned(bound) = trial {
        *env = Cow::Owned(bound);
    }
    Some(())
}
