use super::AstMatcher;
use super::binds::{BindingFilter, capture_range};
use super::facts::{Facts, Target, slot};
use super::relation::may_bind;
use super::resolve::{file_table, file_usage, table_reading, unparenthesized};
use super::spend_util_step;
use super::util::{THREE_VALUED, UNKNOWN};
use crate::code_rule_dsl::identifier::name_in;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::language::adapter::{Adapter, ScopeRules, with_adapter};
use crate::code_rule_dsl::scope::Lookup;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::scope::{Reference, ScopeTable};
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::matcher::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
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

thread_local! {
    /// The binding nodes whose sub-rules are being matched, with the sub-rule matching each: a
    /// chain that comes back to one (`var a = b, b = a`, `var x = x`) binds nothing and fails.
    static FOLLOWING: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
}

/// The binding sub-rules being matched (`FOLLOWING`), which a verdict reads: it fails where it
/// comes back to one of them.
pub(super) fn following_key() -> Vec<(usize, usize)> {
    FOLLOWING.with_borrow(Clone::clone)
}

impl BindingTest {
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
        let name = name_in(self.language, reference.text());
        if undecided(&table, found) {
            UNKNOWN.set(true);
            return None;
        }
        let (binding, value) = table.binding_by(found, &name, self.lookup)?;
        let (scope, index) = resolved_index(&table, found, &name, self.lookup)?;
        let id = crate::code_rule_dsl::scope::usage::BindingId {
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
                crate::code_rule_dsl::scope::usage::BindingId {
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
    with_adapter!(table.family, A => <<A as Adapter>::Scope as ScopeRules>::index(table, reference, name, lookup))
}

/// Whether a read's binding is not known, though some binding may reach it (PHP's flow: several
/// may, a path may reach none, or PHP's dynamic symbol table may decide).
fn undecided(table: &ScopeTable, reference: &Reference) -> bool {
    with_adapter!(table.family, A => <<A as Adapter>::Scope as ScopeRules>::undecided(table, reference))
}

/// Matches `rule` at the first of `occurrences` (nodes of `anchor`'s file) where it matches,
/// committing what it binds; `None` when it matches at none. Three-valued as a relation's search:
/// an unknown occurrence leaves the answer open unless a later one matches, and ends the search
/// when `rule` binds captures, since which occurrence binds them first is then unknown too.
pub(super) fn match_occurrence<'tree, D: Doc>(
    anchor: &Node<'tree, D>,
    rule: &AstMatcher,
    occurrences: &[NodeRef],
    env: &mut Cow<MetaVarEnv<'tree, D>>,
) -> Option<()> {
    let three = THREE_VALUED.get();
    let saved = three && UNKNOWN.replace(false);
    let mut unknown = false;
    let mut strict = None;
    let root = anchor.root().root();
    for occurrence in occurrences {
        if !spend_util_step() {
            break;
        }
        let Some(target) = occurrence.find_near(root.clone(), anchor) else {
            continue;
        };
        let mut trial = Cow::Borrowed(env.as_ref());
        if rule.match_node_with_env(target, &mut trial).is_some()
            && !(super::each::active() && super::each::excluded(&trial))
        {
            if three {
                UNKNOWN.set(saved);
            }
            if let Cow::Owned(bound) = trial {
                *env = Cow::Owned(bound);
            }
            return Some(());
        }
        if three && UNKNOWN.replace(false) {
            unknown = true;
            if *strict.get_or_insert_with(|| may_bind(rule)) {
                break;
            }
        }
    }
    if three {
        UNKNOWN.set(saved || unknown);
    }
    None
}

/// Matches each listed sub-rule against its binding node in `anchor`'s file, all in one trial
/// environment committed only when every one matches. A sub-rule whose node the binding lacks does
/// not match; a chain that comes back to a node its sub-rule is matching fails there.
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
        let target = target.find_near(root.clone(), anchor)?;
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
        let name = name_in(self.language, reference.text());
        if undecided(&table, found) {
            facts.gate(-1, Target::None);
            return;
        }
        let Some((binding, value)) = table.binding_by(found, &name, self.lookup) else {
            return;
        };
        let Some((scope, index)) = resolved_index(&table, found, &name, self.lookup) else {
            return;
        };
        let id = crate::code_rule_dsl::scope::usage::BindingId {
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
                crate::code_rule_dsl::scope::usage::BindingId {
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
                target
                    .find_near(root.clone(), anchor)
                    .map_or(Target::None, Target::Node),
            ),
        };
    }
}

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
                .find_near(root.clone(), anchor)
                .map_or(Target::None, Target::Node),
        );
    }
}
