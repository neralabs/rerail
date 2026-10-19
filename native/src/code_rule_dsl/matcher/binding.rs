use super::AstMatcher;
use super::binds::{BindingFilter, capture_range};
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

/// What `resolves.binding` reads of a node before its sub-rules (`BindingTest::resolve`): the
/// node the binding's nodes are found near, and the binding's site, value and annotation, and
/// its variable's occurrences where an `occurrence` rule reads them.
pub(crate) enum Resolution<'tree, D: Doc> {
    No,
    Unknown,
    Found {
        anchor: Node<'tree, D>,
        site: Option<NodeRef>,
        value: Option<NodeRef>,
        annotation: Option<NodeRef>,
        occurrences: Vec<NodeRef>,
    },
}

impl BindingTest {
    pub(super) fn match_node<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        let (anchor, site, value, annotation, occurrences) = match self.resolve(&node, env.as_ref())
        {
            Resolution::No => return None,
            Resolution::Unknown => {
                UNKNOWN.set(true);
                return None;
            }
            Resolution::Found {
                anchor,
                site,
                value,
                annotation,
                occurrences,
            } => (anchor, site, value, annotation, occurrences),
        };
        let targets = [
            (&self.site, site),
            (&self.value, value),
            (&self.annotation, annotation),
        ];
        let mut trial = Cow::Borrowed(env.as_ref());
        match_binding_nodes(&anchor, targets, &mut trial)?;
        if let Some(rule) = &self.occurrence {
            match_occurrence(&anchor, rule, &occurrences, &mut trial)?;
        }
        if let Cow::Owned(bound) = trial {
            *env = Cow::Owned(bound);
        }
        Some(node)
    }

    /// The binding `node` resolves to, through the filters (which read `env`'s captures), before
    /// any sub-rule: the matcher's and the SQL engine's first step (its sub-rules are relations
    /// over these nodes there).
    pub(crate) fn resolve<'tree, D: Doc>(
        &self,
        node: &Node<'tree, D>,
        env: &MetaVarEnv<'tree, D>,
    ) -> Resolution<'tree, D> {
        let reference = unparenthesized(&self.paren_kinds, node.clone());
        if self.write_last {
            return self.resolve_anchor(reference, env);
        }
        if !self.reference_kinds.contains(&reference.kind_id()) {
            return Resolution::No;
        }
        let Some(table) = file_table(
            &reference,
            self.language,
            table_reading(self.options.as_deref(), self.python.as_deref()),
        ) else {
            return Resolution::No;
        };
        let Some(found) = table.reference(&reference.range()) else {
            return Resolution::No;
        };
        let name = name_in(self.language, reference.text());
        if undecided(&table, found) {
            return Resolution::Unknown;
        }
        let Some((binding, value)) = table.binding_by(found, &name, self.lookup) else {
            return Resolution::No;
        };
        let Some((scope, index)) = resolved_index(&table, found, &name, self.lookup) else {
            return Resolution::No;
        };
        let id = crate::code_rule_dsl::scope::usage::BindingId {
            scope,
            name: &name,
            index,
        };
        let usage = if self.filter.reads_usage() || self.occurrence.is_some() {
            match file_usage(&reference, &table) {
                Some(usage) => Some(usage),
                None => return Resolution::No,
            }
        } else {
            None
        };
        let region = |capture: &str| capture_range(env, capture);
        match self.filter.verdict(&table, usage.as_deref(), id, &region) {
            Some(true) => {}
            Some(false) => return Resolution::No,
            None => return Resolution::Unknown,
        }
        let occurrences = match (&self.occurrence, &usage) {
            (Some(_), Some(usage)) => usage.occurrences(id).to_vec(),
            _ => Vec::new(),
        };
        Resolution::Found {
            site: binding.site.node,
            value: self.value_of(binding.site.node, value),
            annotation: binding.site.annotation,
            occurrences,
            anchor: reference,
        }
    }

    /// `resolves.write: "last"`: `reference` (the node without parentheses) is its variable's
    /// anchor, and the variable passes the filters.
    fn resolve_anchor<'tree, D: Doc>(
        &self,
        reference: Node<'tree, D>,
        env: &MetaVarEnv<'tree, D>,
    ) -> Resolution<'tree, D> {
        let kind = reference.kind_id();
        if !self.reference_kinds.contains(&kind) && !self.name_kinds.contains(&kind) {
            return Resolution::No;
        }
        let Some(table) = file_table(
            &reference,
            self.language,
            table_reading(self.options.as_deref(), self.python.as_deref()),
        ) else {
            return Resolution::No;
        };
        let Some(usage) = file_usage(&reference, &table) else {
            return Resolution::No;
        };
        let range = reference.range();
        let name = reference.text();
        let id = match table.reference(&range) {
            Some(found) => {
                let Some((scope, index)) = table.resolve_lexical_index(found, &name) else {
                    return Resolution::No;
                };
                crate::code_rule_dsl::scope::usage::BindingId {
                    scope,
                    name: &name,
                    index,
                }
            }
            None => match usage.named_at(range.start, range.end, kind).next() {
                Some(id) => id,
                None => return Resolution::No,
            },
        };
        let anchor = (range.start as u32, range.end as u32);
        if usage.facts(id).anchor != Some(anchor)
            || (self.lookup.only && table.chain(id).len() != 1)
        {
            return Resolution::No;
        }
        let region = |capture: &str| capture_range(env, capture);
        match self.filter.verdict(&table, Some(&usage), id, &region) {
            Some(true) => {}
            Some(false) => return Resolution::No,
            None => return Resolution::Unknown,
        }
        let binding = table.binding_of(id);
        let definitions = table.chain(id).len();
        let value = binding
            .site
            .value
            .filter(|_| definitions == 1 && binding.writes == 0);
        let occurrences = if self.occurrence.is_some() {
            usage.occurrences(id).to_vec()
        } else {
            Vec::new()
        };
        Resolution::Found {
            site: binding.site.node,
            value: self.value_of(binding.site.node, value),
            annotation: binding.site.annotation,
            occurrences,
            anchor: reference,
        }
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
