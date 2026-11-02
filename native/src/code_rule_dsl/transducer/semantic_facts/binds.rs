//! `binds` (Python) as a fact producer (O24; decision 0011).
//!
//! One pass per document and table reading emits the relations every form of the predicate
//! reads, whatever clauses read them:
//! - `names`: each binding name node and the bindings it names, in the table's order;
//! - `bindings`: each such binding's facts (form, scope, whether it is its name's latest and made
//!   under `global`/`nonlocal`, whether reads use it, its site, value and annotation nodes);
//! - `shadows`: the binding each one shadows (Ruff's `shadowed_bindings` pair), whose facts are
//!   rows of `bindings` too;
//! - `chains`: each scope's bindings of a name, by their name nodes (`bindings`, `within`).
//!
//! A clause's form is then a filter over these rows (`label`), and its sub-rules (`site`,
//! `value`, `annotation`, `redefines`') are labels of their own that the label joins on the
//! binding's nodes. W's matcher (`Binds::match_node`) is the oracle (`gate`).

// Read only by the gates until a lowering reads the labels (O41).
#![allow(dead_code)]

use super::scopes::Scopes;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::language::adapter::{Python, ScopeRules};
use crate::code_rule_dsl::matcher::{AstMatcher, BindingFilter, Binds, ScopeName, node_at};
use crate::code_rule_dsl::scope::usage::BindingId;
use crate::code_rule_dsl::scope::{Form, ScopeKind, ScopeTable};
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::{AstGrep as Root, Node};
use std::collections::{BTreeMap, BTreeSet};

/// A binding: its scope, name and index among the scope's bindings of the name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    pub(crate) scope: u32,
    pub(crate) name: Box<str>,
    pub(crate) index: usize,
}

impl Key {
    fn of(id: BindingId<'_>) -> Self {
        Self {
            scope: id.scope,
            name: id.name.into(),
            index: id.index,
        }
    }

    fn id(&self) -> BindingId<'_> {
        BindingId {
            scope: self.scope,
            name: &self.name,
            index: self.index,
        }
    }
}

/// A binding's node (site, value or annotation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// The binding has none.
    Absent,
    /// It has one the document's tree does not hold.
    Lost,
    Node(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BindingFacts {
    pub(crate) form: Form,
    pub(crate) scope: ScopeKind,
    /// The live binding of its name in its scope (Ruff's `scope.bindings()`).
    pub(crate) latest: bool,
    /// Made under a `global` or `nonlocal` declaration.
    pub(crate) outer: bool,
    /// The usage index's facts: `None` is unknown.
    pub(crate) used: Option<bool>,
    pub(crate) referenced: Option<bool>,
    pub(crate) nested_use: Option<bool>,
    pub(crate) exported: Option<bool>,
    /// Ruff's branch path.
    pub(crate) branch: u32,
    /// The qualified name it imports, as Ruff compares imports (`ScopeTable::imported`).
    pub(crate) imported: Option<String>,
    pub(crate) site: Target,
    pub(crate) value: Target,
    pub(crate) annotation: Target,
}

/// A scope's bindings of a name: how many, and the byte ranges of their name (or site) nodes.
pub(crate) type Chain = (usize, Vec<(u32, u32)>);

/// One document's rows.
#[derive(Debug, Default)]
pub(crate) struct BindsRows {
    /// (name node, its kind, a binding it names), by node, then in the table's order.
    pub(crate) names: Vec<(u32, u16, Key)>,
    pub(crate) bindings: BTreeMap<Key, BindingFacts>,
    /// A binding, the one it shadows, and whether that one is of the same scope.
    pub(crate) shadows: BTreeMap<Key, (Key, bool)>,
    /// Each scope's bindings of a name, by scope and name.
    pub(crate) chains: BTreeMap<(u32, Box<str>), Chain>,
}

impl BindsRows {
    pub(crate) fn len(&self) -> usize {
        self.names.len() + self.bindings.len() + self.shadows.len() + self.chains.len()
    }
}

fn target<'r>(
    root: &'r Root<TapeDoc>,
    anchor: &Node<'r, TapeDoc>,
    node: Option<NodeRef>,
) -> Target {
    match node {
        None => Target::Absent,
        Some(node) => node
            .find_near(root.root(), anchor)
            .map_or(Target::Lost, |found| Target::Node(found.node_id() as u32)),
    }
}

fn facts<'r>(
    table: &ScopeTable,
    scopes: &Scopes,
    root: &'r Root<TapeDoc>,
    anchor: &Node<'r, TapeDoc>,
    key: &Key,
) -> BindingFacts {
    let id = key.id();
    let binding = table.binding_of(id);
    let usage = scopes.usage.facts(id);
    BindingFacts {
        form: binding.site.form,
        scope: table.scopes[key.scope as usize].kind,
        latest: key.index + 1 == table.chain(id).len(),
        outer: binding.outer,
        used: usage.used,
        referenced: usage.referenced,
        nested_use: usage.nested_use,
        exported: usage.exported,
        branch: binding.branch,
        imported: ScopeTable::imported(binding),
        site: target(root, anchor, binding.site.node),
        value: target(root, anchor, binding.site.value),
        annotation: target(root, anchor, binding.site.annotation),
    }
}

/// The rows of `root`'s document, whose table and usage index are `scopes`.
pub(crate) fn rows(root: &Root<TapeDoc>, scopes: &Scopes) -> BindsRows {
    let table = &scopes.table;
    let tape = root.root().get_doc();
    let mut rows = BindsRows::default();
    for ordinal in 0..tape.transducer_len() as u32 {
        let span = tape.transducer_span(ordinal);
        let kind = tape.transducer_kind(ordinal);
        let mut named = scopes.usage.named_at(span.start, span.end, kind).peekable();
        if named.peek().is_none() {
            continue;
        }
        let anchor = node_at(root, ordinal);
        for id in named {
            let key = Key::of(id);
            let mut pending = vec![key.clone()];
            if let Some((shadowed, same_scope)) = table.shadowed(id) {
                let shadowed = Key::of(shadowed);
                pending.push(shadowed.clone());
                rows.shadows.insert(key.clone(), (shadowed, same_scope));
            }
            for key in pending {
                if rows.bindings.contains_key(&key) {
                    continue;
                }
                let chain = (key.scope, key.name.clone());
                rows.chains.entry(chain).or_insert_with(|| {
                    let bindings = table.chain(key.id());
                    let names = bindings
                        .iter()
                        .filter_map(|binding| binding.name.or(binding.site.node))
                        .map(|name| (name.start, name.end))
                        .collect();
                    (bindings.len(), names)
                });
                let facts = facts(table, scopes, root, &anchor, &key);
                rows.bindings.insert(key, facts);
            }
            rows.names.push((ordinal, kind, key));
        }
    }
    rows
}

/// A form this producer's rows do not answer, named.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Refused(pub(crate) &'static str);

fn admits_filter(filter: &BindingFilter) -> Result<(), Refused> {
    if filter.options.is_some() || filter.directives || filter.builtin.is_some() {
        return Err(Refused("languageOptions"));
    }
    Ok(())
}

/// Whether the rows answer `form`: Python's, without TypeScript's `occurrence`.
pub(crate) fn admits(form: &Binds) -> Result<(), Refused> {
    if form.language != GrammarDialect::Python {
        return Err(Refused("not Python"));
    }
    if form.occurrence.is_some() {
        return Err(Refused("occurrence"));
    }
    admits_filter(&form.filter)?;
    if let Some(redefines) = &form.redefines {
        if redefines.every {
            return Err(Refused("redefines.every"));
        }
        admits_filter(&redefines.filter)?;
    }
    Ok(())
}

/// A three-valued verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    True,
    False,
    Unknown,
}

/// The byte range of a capture the clause bound where the label is read (`bindings.within`).
pub(crate) type Region<'a> = &'a dyn Fn(&str) -> Option<(usize, usize)>;

/// Whether a binding passes `filter` (W's `BindingFilter::verdict`, Python's reading): `None`
/// where a fact it reads is unknown and nothing else fails.
fn passes(
    filter: &BindingFilter,
    rows: &BindsRows,
    key: &Key,
    facts: &BindingFacts,
    region: Region<'_>,
) -> Option<bool> {
    let holds = |wanted: Option<bool>, actual: bool| wanted.is_none_or(|wanted| wanted == actual);
    let count = filter.bindings.as_ref().is_none_or(|count| {
        let (len, names) = &rows.chains[&(key.scope, key.name.clone())];
        let n = match &count.within {
            None => *len,
            Some(capture) => region(capture).map_or(0, |(start, end)| {
                names
                    .iter()
                    .filter(|(name_start, name_end)| {
                        start <= *name_start as usize && *name_end as usize <= end
                    })
                    .count()
            }),
        };
        count.min <= n && count.max.is_none_or(|max| n <= max)
    });
    if !filter
        .forms
        .as_ref()
        .is_none_or(|forms| forms.contains(&facts.form))
        || !filter
            .scopes
            .as_ref()
            .is_none_or(|scopes| scopes.contains(&ScopeName::Kind(facts.scope)))
        || !holds(filter.latest, facts.latest)
        || !holds(filter.outer, facts.outer)
        || !count
    {
        return Some(false);
    }
    if !filter.reads_usage() {
        return Some(true);
    }
    let mut unknown = false;
    for (wanted, actual) in [
        (filter.used, facts.used),
        (filter.referenced, facts.referenced),
        (filter.nested_use, facts.nested_use),
        (filter.exported, facts.exported),
    ] {
        let Some(wanted) = wanted else { continue };
        match actual {
            Some(actual) if actual != wanted => return Some(false),
            Some(_) => {}
            None => unknown = true,
        }
    }
    (!unknown).then_some(true)
}

/// The sub-rules on a binding's nodes, in W's order (site, value, annotation): the first that
/// does not hold decides. A rule on a node the binding lacks fails before any is read.
fn nodes_match(
    rules: [(&Option<AstMatcher>, Target); 3],
    sub_rule: &mut dyn FnMut(&AstMatcher, u32) -> Verdict,
) -> Verdict {
    let rules = rules
        .into_iter()
        .filter_map(|(rule, target)| rule.as_ref().map(|rule| (rule, target)))
        .collect::<Vec<_>>();
    if rules.iter().any(|(_, target)| *target == Target::Absent) {
        return Verdict::False;
    }
    for (rule, target) in rules {
        let Target::Node(node) = target else {
            return Verdict::False;
        };
        match sub_rule(rule, node) {
            Verdict::True => {}
            other => return other,
        }
    }
    Verdict::True
}

/// The verdict of `form` at each name node of `rows`' document: its label, as (true, unknown)
/// node sets. `sub_rule` gives a sub-rule's verdict at a node (its own label); `region` the
/// captures the clause bound.
pub(crate) fn label(
    form: &Binds,
    rows: &BindsRows,
    sub_rule: &mut dyn FnMut(&AstMatcher, u32) -> Verdict,
    region: Region<'_>,
) -> Result<(BTreeSet<u32>, BTreeSet<u32>), Refused> {
    admits(form)?;
    // Python's `redefines` reads the binding each one shadows, not the variable's others.
    const { assert!(<Python as ScopeRules>::REDEFINES_SHADOWED) };
    let mut verdicts = BTreeMap::<u32, Verdict>::new();
    for (node, kind, key) in &rows.names {
        if !form.name_kinds.contains(kind) {
            continue;
        }
        let verdict = verdicts.entry(*node).or_insert(Verdict::False);
        if *verdict == Verdict::True {
            continue;
        }
        let facts = &rows.bindings[key];
        let binding = match passes(&form.filter, rows, key, facts, region) {
            Some(true) => Verdict::True,
            Some(false) => Verdict::False,
            None => Verdict::Unknown,
        };
        let binding = if binding != Verdict::True {
            binding
        } else {
            let shadowed = match form.redefines.as_deref() {
                None => Ok(None),
                Some(redefines) => match rows.shadows.get(key) {
                    None => Err(Verdict::False),
                    Some((shadowed, same_scope)) => {
                        let previous = &rows.bindings[shadowed];
                        let same_import = facts
                            .imported
                            .as_ref()
                            .is_some_and(|name| previous.imported.as_ref() == Some(name));
                        if facts.branch != previous.branch
                            || redefines
                                .same_scope
                                .is_some_and(|wanted| wanted != *same_scope)
                            || redefines
                                .same_import
                                .is_some_and(|wanted| wanted != same_import)
                        {
                            Err(Verdict::False)
                        } else {
                            match passes(&redefines.filter, rows, shadowed, previous, region) {
                                Some(true) => Ok(Some((redefines, previous))),
                                Some(false) => Err(Verdict::False),
                                None => Err(Verdict::Unknown),
                            }
                        }
                    }
                },
            };
            match shadowed {
                Err(verdict) => verdict,
                Ok(shadowed) => {
                    let own = nodes_match(
                        [
                            (&form.site, facts.site),
                            (&form.value, facts.value),
                            (&form.annotation, facts.annotation),
                        ],
                        sub_rule,
                    );
                    match (own, shadowed) {
                        (Verdict::True, Some((redefines, previous))) => nodes_match(
                            [
                                (&redefines.site, previous.site),
                                (&redefines.value, previous.value),
                                (&redefines.annotation, previous.annotation),
                            ],
                            sub_rule,
                        ),
                        (own, _) => own,
                    }
                }
            }
        };
        if binding != Verdict::False {
            *verdict = binding;
        }
    }
    let pick = |wanted: Verdict| {
        verdicts
            .iter()
            .filter(move |(_, verdict)| **verdict == wanted)
            .map(|(node, _)| *node)
            .collect()
    };
    Ok((pick(Verdict::True), pick(Verdict::Unknown)))
}
