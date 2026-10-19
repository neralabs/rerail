use super::AstMatcher;
use super::binding::{match_binding_nodes, match_occurrence};
use super::resolve::{file_table, file_usage, table_reading};
use super::util::{THREE_VALUED, UNKNOWN};
use crate::code_rule_dsl::language::adapter::{Adapter, ScopeRules, with_adapter};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::scope::usage::{BindingId, PrivateMember, Usage};
use crate::code_rule_dsl::scope::{Form, ScopeKind, ScopeTable, TYPE, VALUE};
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

/// A scope a filter names: a table scope kind, or (TypeScript) eslint-scope's global scope, a
/// script's top level, told apart from a module's (`ScopeKind::Module`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ScopeName {
    Kind(ScopeKind),
    Global,
}

/// Filters over a binding: how and where it binds, and whether it is read.
#[derive(Clone, Default)]
pub(crate) struct BindingFilter {
    /// The binding's form is one of these.
    pub(crate) forms: Option<Vec<Form>>,
    /// The scope that holds it is one of these.
    pub(crate) scopes: Option<Vec<ScopeName>>,
    /// Some read resolves to it (with what it inherits); unknown where a read may be unseen.
    pub(crate) used: Option<bool>,
    /// Some reference resolves to it, writes included.
    pub(crate) referenced: Option<bool>,
    /// Some such read comes from another scope.
    pub(crate) nested_use: Option<bool>,
    /// Python: an explicit re-export, or named by a static module `__all__`. TypeScript: a
    /// definition is exported (with `directives`, or a global `/* exported */` comment names it).
    pub(crate) exported: Option<bool>,
    /// It is the live binding of its name in its scope (Ruff's `scope.bindings()`).
    pub(crate) latest: Option<bool>,
    /// It is made under a `global` or `nonlocal` declaration.
    pub(crate) outer: Option<bool>,
    /// How many bindings of its name its scope holds, within a captured node or in all.
    pub(crate) bindings: Option<BindingCount>,
    /// TypeScript: it is the global variable of a configured global, in a script's top level
    /// (ESLint's `eslintImplicitGlobalSetting`, which no-redeclare's `builtinGlobals` reads).
    pub(crate) builtin: Option<bool>,
    /// TypeScript `directives: "eslint"`: `/* exported */` comments export global variables.
    pub(crate) directives: bool,
    pub(crate) language: Option<GrammarDialect>,
    /// ESLint's settings declaring the family's globals (`ScopeRules::eslint_options`): the
    /// query's `languageOptions` (script or module, the JSX pragma), else ESLint's defaults;
    /// `None` for a family they do not configure.
    pub(crate) options: Option<Arc<LanguageOptions>>,
}

/// `bindings`: the bindings of the binding's name in its scope (Ruff's `scope.get_all`, deletions
/// included), counted where their name node lies inside the node captured as `within`.
#[derive(Clone, Debug, Default)]
pub(crate) struct BindingCount {
    /// The capture whose node bounds the count; `None` counts the whole scope.
    pub(crate) within: Option<String>,
    pub(crate) min: usize,
    pub(crate) max: Option<usize>,
}

/// The byte range of a capture in the environment a filter runs in.
pub(crate) type Region<'a> = &'a dyn Fn(&str) -> Option<(usize, usize)>;

impl BindingCount {
    /// `holds` for a private member: its declarations.
    fn holds_private(&self, member: &PrivateMember, region: Region<'_>) -> bool {
        let count = match &self.within {
            None => member.declarations.len(),
            Some(capture) => region(capture).map_or(0, |(start, end)| {
                member
                    .declarations
                    .iter()
                    .filter(|declaration| {
                        start <= declaration.name.start as usize
                            && declaration.name.end as usize <= end
                    })
                    .count()
            }),
        };
        self.min <= count && self.max.is_none_or(|max| count <= max)
    }

    fn holds(&self, table: &ScopeTable, id: BindingId<'_>, region: Region<'_>) -> bool {
        let count = match &self.within {
            None => table.chain(id).len(),
            Some(capture) => region(capture).map_or(0, |(start, end)| {
                table
                    .chain(id)
                    .iter()
                    .filter_map(|binding| binding.name.or(binding.site.node))
                    .filter(|name| start <= name.start as usize && name.end as usize <= end)
                    .count()
            }),
        };
        self.min <= count && self.max.is_none_or(|max| count <= max)
    }
}

/// `region` over a matcher's environment: a single-node capture's range.
pub(crate) fn capture_range<D: Doc>(
    env: &MetaVarEnv<'_, D>,
    capture: &str,
) -> Option<(usize, usize)> {
    env.get_match(capture).map(|node| {
        let range = node.range();
        (range.start, range.end)
    })
}

/// The filters without the language options, whose globals table would swamp a profile.
impl std::fmt::Debug for BindingFilter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BindingFilter")
            .field("forms", &self.forms)
            .field("scopes", &self.scopes)
            .field("used", &self.used)
            .field("referenced", &self.referenced)
            .field("nested_use", &self.nested_use)
            .field("exported", &self.exported)
            .field("latest", &self.latest)
            .field("outer", &self.outer)
            .field("bindings", &self.bindings)
            .field("builtin", &self.builtin)
            .field("directives", &self.directives)
            .finish()
    }
}

impl BindingFilter {
    /// Whether a filter reads the usage index.
    pub(crate) fn reads_usage(&self) -> bool {
        self.used.is_some()
            || self.referenced.is_some()
            || self.nested_use.is_some()
            || self.exported.is_some()
    }

    /// The name a filter gives the scope at `index`.
    pub(crate) fn scope_name(&self, table: &ScopeTable, index: u32) -> ScopeName {
        let kind = table.scopes[index as usize].kind;
        let global = index == 0
            && self
                .options
                .as_deref()
                .is_some_and(LanguageOptions::global_top_level);
        if global {
            ScopeName::Global
        } else {
            ScopeName::Kind(kind)
        }
    }

    /// Whether the binding's variable is a configured global's: a script's top level, and the
    /// query's `languageOptions` declare the name; with `directives`, or a `/* global */` comment
    /// names it.
    fn builtin(&self, table: &ScopeTable, id: BindingId<'_>) -> bool {
        let commented = self.directives
            && (table.directives.declared.contains(id.name)
                || table.directives.off.contains(id.name));
        self.scope_name(table, id.scope) == ScopeName::Global
            && (commented
                || self
                    .options
                    .as_deref()
                    .is_some_and(|options| options.declares_in(id.name, VALUE | TYPE, None)))
    }

    /// Whether typescript-eslint's JSX pragma (or, for fragments, fragment name) reads the variable
    /// `id` belongs to: the first JSX element whose scope sees a variable of that name reads it.
    fn pragma_reads(&self, table: &ScopeTable, id: BindingId<'_>) -> bool {
        let Some(options) = self.options.as_deref() else {
            return false;
        };
        if table.jsx.is_empty() {
            return false;
        }
        let pragma = options.jsx_pragma.as_deref();
        let fragment = options.jsx_fragment_name.as_deref();
        [(pragma, false), (fragment, true)]
            .into_iter()
            .any(|(name, fragments_only)| {
                name == Some(id.name)
                    && pragma_variable(table, id.name, fragments_only) == Some(id.scope)
            })
    }

    /// Whether declaration `index` of a private member passes. Its scope is its class body's
    /// (`class`); it is never exported nor a configured global; `nestedUse` is unknown.
    fn private_verdict(
        &self,
        member: &PrivateMember,
        index: usize,
        region: Region<'_>,
    ) -> Option<bool> {
        let holds =
            |wanted: Option<bool>, actual: bool| wanted.is_none_or(|wanted| wanted == actual);
        let form = member.declarations[index].form;
        if !self
            .forms
            .as_ref()
            .is_none_or(|forms| forms.contains(&form))
            || !self
                .scopes
                .as_ref()
                .is_none_or(|scopes| scopes.contains(&ScopeName::Kind(ScopeKind::Class)))
            || !holds(self.exported, false)
            || !holds(self.builtin, false)
            || !holds(self.used, member.used)
            || !holds(self.referenced, member.referenced)
            || !self
                .bindings
                .as_ref()
                .is_none_or(|count| count.holds_private(member, region))
        {
            return Some(false);
        }
        self.nested_use.is_none().then_some(true)
    }

    /// Whether the binding passes: `None` when a fact it reads is unknown and nothing else fails.
    /// PHP reads `latest` and `outer` from its variables' flow, where a path may decide them.
    pub(crate) fn verdict(
        &self,
        table: &ScopeTable,
        usage: Option<&Usage>,
        id: BindingId<'_>,
        region: Region<'_>,
    ) -> Option<bool> {
        let binding = table.binding_of(id);
        let holds =
            |wanted: Option<bool>, actual: bool| wanted.is_none_or(|wanted| wanted == actual);
        // The flow keys a binding by where it is made: its name node, or a Blade marker.
        let php = table.php.as_deref().filter(|_| {
            self.language
                .is_some_and(|language| flow(Family::of(language)))
        });
        let (latest, outer) = match php {
            Some(flow) => {
                let start = binding.offset as u32;
                (flow.latest(id.scope, start), flow.outer(start))
            }
            None => (
                Some(id.index + 1 == table.chain(id).len()),
                Some(binding.outer),
            ),
        };
        let fails = |wanted: Option<bool>, actual: Option<bool>| {
            wanted.is_some_and(|wanted| actual.is_some_and(|actual| actual != wanted))
        };
        if !self
            .forms
            .as_ref()
            .is_none_or(|forms| forms.contains(&binding.site.form))
            || !self
                .scopes
                .as_ref()
                .is_none_or(|scopes| scopes.contains(&self.scope_name(table, id.scope)))
            || fails(self.latest, latest)
            || fails(self.outer, outer)
            || !self
                .bindings
                .as_ref()
                .is_none_or(|count| count.holds(table, id, region))
            || !holds(
                self.builtin,
                self.builtin.is_some() && self.builtin(table, id),
            )
        {
            return Some(false);
        }
        let mut unknown = (self.latest.is_some() && latest.is_none())
            || (self.outer.is_some() && outer.is_none());
        let Some(usage) = usage.filter(|_| self.reads_usage()) else {
            return (!unknown).then_some(true);
        };
        let mut facts = usage.facts(id);
        if (self.used.is_some() || self.referenced.is_some()) && self.pragma_reads(table, id) {
            facts.used = Some(true);
            facts.referenced = Some(true);
        }
        if self.directives
            && facts.exported == Some(false)
            && self.scope_name(table, id.scope) == ScopeName::Global
            && usage.comment_exports.contains(id.name)
        {
            facts.exported = Some(true);
        }
        for (wanted, actual) in [
            (self.used, facts.used),
            (self.referenced, facts.referenced),
            (self.nested_use, facts.nested_use),
            (self.exported, facts.exported),
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
}

/// The scope of the variable named `name` that typescript-eslint's `referenceInSomeUpperScope`
/// finds from the first JSX element (or fragment) whose scope chain has one.
fn pragma_variable(table: &ScopeTable, name: &str, fragments_only: bool) -> Option<u32> {
    for (scope, fragment) in &table.jsx {
        if fragments_only && !fragment {
            continue;
        }
        let mut current = Some(*scope);
        while let Some(index) = current {
            let scope = &table.scopes[index as usize];
            if scope.bindings.contains_key(name) {
                return Some(index);
            }
            current = scope.parent;
        }
    }
    None
}

/// `binds.redefines`: the binding shadows one that passes these filters and sub-rules, on the same
/// fork (Ruff's branch path).
#[derive(Clone)]
pub(crate) struct Redefines {
    pub(crate) filter: BindingFilter,
    /// Whether the shadowed binding is of the same scope.
    pub(crate) same_scope: Option<bool>,
    /// TypeScript: every other definition of the variable passes, rather than some earlier one.
    pub(crate) every: bool,
    /// Whether both are imports of the same qualified name.
    pub(crate) same_import: Option<bool>,
    pub(crate) site: Option<AstMatcher>,
    pub(crate) value: Option<AstMatcher>,
    pub(crate) annotation: Option<AstMatcher>,
}

/// `binds` (a Rerail extension): the node is the name node of a binding (where a linter reports
/// it) that passes the filters, and each listed sub-rule matches that binding's node. The
/// sub-rules share the caller's environment, as `has` does, so they may bind captures.
#[derive(Clone)]
pub(crate) struct Binds {
    pub(crate) language: GrammarDialect,
    /// Kinds a name node can have.
    pub(crate) name_kinds: BTreeSet<u16>,
    pub(crate) options: Option<Arc<LanguageOptions>>,
    pub(crate) python: Option<Arc<PythonOptions>>,
    pub(crate) filter: BindingFilter,
    pub(crate) site: Option<AstMatcher>,
    pub(crate) value: Option<AstMatcher>,
    pub(crate) annotation: Option<AstMatcher>,
    /// TypeScript: matches some definition name or reference of the binding's variable.
    pub(crate) occurrence: Option<AstMatcher>,
    pub(crate) redefines: Option<Box<Redefines>>,
}

impl Binds {
    /// The sub-rules, the shadowed binding's included, which may bind captures.
    pub(crate) fn sub_rules(&self) -> impl Iterator<Item = &AstMatcher> {
        let redefines = self.redefines.as_deref();
        [
            &self.site,
            &self.value,
            &self.annotation,
            &self.occurrence,
            redefines.map_or(&None, |redefines| &redefines.site),
            redefines.map_or(&None, |redefines| &redefines.value),
            redefines.map_or(&None, |redefines| &redefines.annotation),
        ]
        .into_iter()
        .flatten()
    }

    /// Whether `id` shadows a binding the redefines term accepts, on the same fork: the shadowed
    /// binding, or `None` when a usage fact the filters read is unknown.
    fn shadowed<'t>(
        redefines: &Redefines,
        table: &'t ScopeTable,
        usage: &Usage,
        id: BindingId<'t>,
        region: Region<'_>,
    ) -> Option<Option<BindingId<'t>>> {
        let Some((shadowed, same_scope)) = table.shadowed(id) else {
            return Some(None);
        };
        let (binding, previous) = (table.binding_of(id), table.binding_of(shadowed));
        let same_import = ScopeTable::imported(binding)
            .is_some_and(|name| ScopeTable::imported(previous).as_ref() == Some(&name));
        if binding.branch != previous.branch
            || redefines
                .same_scope
                .is_some_and(|wanted| wanted != same_scope)
            || redefines
                .same_import
                .is_some_and(|wanted| wanted != same_import)
        {
            return Some(None);
        }
        match redefines
            .filter
            .verdict(table, Some(usage), shadowed, region)?
        {
            true => Some(Some(shadowed)),
            false => Some(None),
        }
    }

    /// The bindings `redefines` reads beside `id`, each with whether `sameScope` admits it, and
    /// whether others may exist the table does not know of; `None` where `id` redefines nothing.
    /// TypeScript: the variable's other definitions (eslint-scope's `identifiers`, `arguments`
    /// aside) in document order, the earlier ones only unless `every`. PHP: the bindings of the
    /// variable some path reaches `id`'s site with (the ones it overwrites, itself in a loop),
    /// whatever `every`; PHP's dynamic symbol table may add others.
    #[allow(clippy::type_complexity)]
    fn others<'t>(
        redefines: &Redefines,
        table: &'t ScopeTable,
        id: BindingId<'t>,
    ) -> Option<(Vec<(BindingId<'t>, bool)>, bool)> {
        if flow(table.family) {
            let start = table.binding_of(id).offset as u32;
            let reach = table.php.as_deref()?.reach(start)?;
            if reach.bindings.is_empty() && !reach.unknown {
                return None;
            }
            let others = reach
                .bindings
                .iter()
                .map(|&(scope, index)| {
                    let same = scope == id.scope;
                    (
                        BindingId { scope, index, ..id },
                        redefines.same_scope.is_none_or(|wanted| wanted == same),
                    )
                })
                .collect();
            return Some((others, reach.unknown));
        }
        if redefines.same_scope == Some(false) {
            return None;
        }
        let others = table
            .chain(id)
            .iter()
            .enumerate()
            .filter(|(index, binding)| {
                binding.name.is_some()
                    && if redefines.every {
                        *index != id.index
                    } else {
                        *index < id.index
                    }
            })
            .map(|(index, _)| (BindingId { index, ..id }, true))
            .collect();
        Some((others, false))
    }

    /// TypeScript's and PHP's `redefines` over `Binds::others`: some other binding passes the
    /// filters and sub-rules, or, with `every`, each one does. `None` when a fact decides and is
    /// unknown, or a binding the table does not know of might.
    #[allow(clippy::too_many_arguments)]
    fn redefines_others<'tree, D: Doc>(
        redefines: &Redefines,
        table: &ScopeTable,
        usage: &Usage,
        id: BindingId<'_>,
        node: &Node<'tree, D>,
        trial: &mut Cow<MetaVarEnv<'tree, D>>,
        region: Region<'_>,
    ) -> Option<bool> {
        let Some((others, incomplete)) = Self::others(redefines, table, id) else {
            return Some(false);
        };
        let mut unknown = false;
        for (other, admitted) in others {
            let passes = if admitted {
                match redefines.filter.verdict(table, Some(usage), other, region) {
                    Some(passes) => passes,
                    None => {
                        unknown = true;
                        false
                    }
                }
            } else {
                false
            };
            let previous = table.binding_of(other);
            let targets = [
                (&redefines.site, previous.site.node),
                (&redefines.value, previous.site.value),
                (&redefines.annotation, previous.site.annotation),
            ];
            if redefines.every {
                if !passes || match_binding_nodes(node, targets, trial).is_none() {
                    return if unknown { None } else { Some(false) };
                }
            } else if passes {
                let mut attempt = Cow::Borrowed(trial.as_ref());
                if match_binding_nodes(node, targets, &mut attempt).is_some() {
                    if super::each::active() && super::each::excluded(&attempt) {
                        continue;
                    }
                    if let Cow::Owned(bound) = attempt {
                        *trial = Cow::Owned(bound);
                    }
                    return Some(true);
                }
            }
        }
        if incomplete || unknown {
            None
        } else {
            Some(redefines.every)
        }
    }

    /// A private name's declaration `index` of `member`: the filters, the sub-rules on its
    /// definition, and `redefines` over the member's other declarations.
    fn match_private<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
        member: &PrivateMember,
        index: usize,
    ) -> Option<Node<'tree, D>> {
        let caller: &MetaVarEnv<'tree, D> = env.as_ref();
        let region = |capture: &str| capture_range(caller, capture);
        let unknown = || {
            UNKNOWN.set(true);
            None
        };
        match self.filter.private_verdict(member, index, &region) {
            Some(true) => {}
            Some(false) => return None,
            None => return unknown(),
        }
        let targets = |index: usize, site, value, annotation| {
            let declaration = &member.declarations[index];
            [
                (site, Some(declaration.site)),
                (value, declaration.value),
                (annotation, declaration.annotation),
            ]
        };
        let mut trial = Cow::Borrowed(env.as_ref());
        match_binding_nodes(
            &node,
            targets(index, &self.site, &self.value, &self.annotation),
            &mut trial,
        )?;
        if let Some(rule) = &self.occurrence {
            match_occurrence(&node, rule, &member.occurrences, &mut trial)?;
        }
        if let Some(redefines) = self.redefines.as_deref() {
            // The member's other declarations, as TypeScript's variables: some earlier one passes,
            // or with `every` each other one does. All are of the same class body. One whose
            // sub-rules are unknown leaves the answer open unless a later one passes.
            if redefines.same_scope == Some(false) {
                return None;
            }
            let others = (0..member.declarations.len()).filter(|other| {
                if redefines.every {
                    *other != index
                } else {
                    *other < index
                }
            });
            let three = THREE_VALUED.get();
            let saved = three && UNKNOWN.replace(false);
            let mut open = false;
            let mut found = redefines.every;
            for other in others {
                let passes = match redefines.filter.private_verdict(member, other, &region) {
                    Some(passes) => passes,
                    None => return unknown(),
                };
                let nodes = targets(
                    other,
                    &redefines.site,
                    &redefines.value,
                    &redefines.annotation,
                );
                let mut attempt = Cow::Borrowed(trial.as_ref());
                let matched = passes && match_binding_nodes(&node, nodes, &mut attempt).is_some();
                if redefines.every && !matched {
                    if saved {
                        UNKNOWN.set(true);
                    }
                    return None;
                }
                if matched {
                    if super::each::active() && super::each::excluded(&attempt) {
                        continue;
                    }
                    if let Cow::Owned(bound) = attempt {
                        trial = Cow::Owned(bound);
                    }
                    if !redefines.every {
                        found = true;
                        break;
                    }
                } else if three && UNKNOWN.replace(false) {
                    open = true;
                }
            }
            if three {
                UNKNOWN.set(saved || (open && !found));
            }
            if !found {
                return None;
            }
        }
        if let Cow::Owned(bound) = trial {
            *env = Cow::Owned(bound);
        }
        Some(node)
    }

    pub(super) fn match_node<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        if !self.name_kinds.contains(&node.kind_id()) {
            return None;
        }
        let table = file_table(
            &node,
            self.language,
            table_reading(self.options.as_deref(), self.python.as_deref()),
        )?;
        let usage = file_usage(&node, &table)?;
        let range = node.range();
        if let Some((member, index)) = usage.private_at(range.start, range.end) {
            return self.match_private(node, env, member, index);
        }
        let three = THREE_VALUED.get();
        let saved = three && UNKNOWN.replace(false);
        let mut unknown = false;
        let caller: &MetaVarEnv<'tree, D> = env.as_ref();
        let region = |capture: &str| capture_range(caller, capture);
        for id in usage.named_at(range.start, range.end, node.kind_id()) {
            // A sub-rule the binding before left unknown is that binding's verdict.
            unknown |= three && UNKNOWN.replace(false);
            match self.filter.verdict(&table, Some(&usage), id, &region) {
                Some(true) => {}
                Some(false) => continue,
                None => {
                    unknown = true;
                    continue;
                }
            }
            let shadowed_only = with_adapter!(table.family, A => <<A as Adapter>::Scope as ScopeRules>::REDEFINES_SHADOWED);
            let shadowed = match self.redefines.as_deref().filter(|_| shadowed_only) {
                None => None,
                Some(redefines) => match Self::shadowed(redefines, &table, &usage, id, &region) {
                    Some(Some(shadowed)) => Some((redefines, shadowed)),
                    Some(None) => continue,
                    None => {
                        unknown = true;
                        continue;
                    }
                },
            };
            let binding = table.binding_of(id);
            let targets = [
                (&self.site, binding.site.node),
                (&self.value, binding.site.value),
                (&self.annotation, binding.site.annotation),
            ];
            let mut trial = Cow::Borrowed(env.as_ref());
            if match_binding_nodes(&node, targets, &mut trial).is_none() {
                continue;
            }
            if let Some(rule) = &self.occurrence
                && match_occurrence(&node, rule, usage.occurrences(id), &mut trial).is_none()
            {
                continue;
            }
            if let Some((redefines, shadowed)) = shadowed {
                let previous = table.binding_of(shadowed);
                let targets = [
                    (&redefines.site, previous.site.node),
                    (&redefines.value, previous.site.value),
                    (&redefines.annotation, previous.site.annotation),
                ];
                if match_binding_nodes(&node, targets, &mut trial).is_none() {
                    continue;
                }
            }
            if let Some(redefines) = self.redefines.as_deref().filter(|_| !shadowed_only) {
                let verdict = Self::redefines_others(
                    redefines, &table, &usage, id, &node, &mut trial, &region,
                );
                match verdict {
                    Some(true) => {}
                    Some(false) => continue,
                    None => {
                        unknown = true;
                        continue;
                    }
                }
            }
            if three {
                UNKNOWN.set(saved);
            }
            if let Cow::Owned(bound) = trial {
                *env = Cow::Owned(bound);
            }
            return Some(node);
        }
        if unknown || saved {
            UNKNOWN.set(true);
        }
        None
    }
}

/// A step's verdict before its sub-rules, as the SQL engine's terminals return it: it passes, it
/// is unknown (its sub-rules are not read), or it fails.
pub(crate) const PASSES: u8 = 0;
pub(crate) const OPEN: u8 = 1;
pub(crate) const FAILS: u8 = 2;

/// One binding a `binds` node names, before its sub-rules (`Binds::candidates`).
pub(crate) struct Candidate {
    /// Its position among the node's bindings (`Usage::named_at`), or `PRIVATE`.
    pub(crate) ordinal: i64,
    /// `OPEN` where its filters (or the binding it shadows) are unknown, else `PASSES`.
    pub(crate) pre: u8,
    /// Its site, value and annotation, where the sub-rule of each is listed.
    pub(crate) nodes: [Option<NodeRef>; 3],
    /// `FAILS` where a node a listed `redefines` sub-rule reads of the shadowed binding is
    /// missing (Python): checked after the binding's own sub-rules.
    pub(crate) shadow: u8,
    /// The shadowed binding's site, value and annotation (Python's `redefines`).
    pub(crate) previous: [Option<NodeRef>; 3],
}

/// A private member's declaration, which `binds` reads alone (`Binds::match_private`).
pub(crate) const PRIVATE: i64 = -1;

/// One binding `redefines` reads beside a candidate (`Binds::others`), before its sub-rules.
pub(crate) struct Other {
    pub(crate) pre: u8,
    pub(crate) nodes: [Option<NodeRef>; 3],
}

/// What the SQL engine's terminals read of `binds`: the matcher's steps before each sub-rule
/// (`Binds::match_node`), the sub-rules being relations over the nodes they return there.
impl Binds {
    /// Whether `redefines` is read through the shadowed binding (Python), not the others.
    pub(crate) fn redefines_shadowed(&self) -> bool {
        with_adapter!(Family::of(self.language), A => <<A as Adapter>::Scope as ScopeRules>::REDEFINES_SHADOWED)
    }

    /// The table and usage at a name node, as `match_node` reads them.
    fn tables<D: Doc>(&self, node: &Node<'_, D>) -> Option<(Arc<ScopeTable>, Arc<Usage>)> {
        if !self.name_kinds.contains(&node.kind_id()) {
            return None;
        }
        let table = file_table(
            node,
            self.language,
            table_reading(self.options.as_deref(), self.python.as_deref()),
        )?;
        let usage = file_usage(node, &table)?;
        Some((table, usage))
    }

    /// The bindings `node` names, in the matcher's order, each with its verdict before its
    /// sub-rules; one that fails is left out.
    pub(crate) fn candidates<'tree, D: Doc>(
        &self,
        node: &Node<'tree, D>,
        env: &MetaVarEnv<'tree, D>,
    ) -> Vec<Candidate> {
        let Some((table, usage)) = self.tables(node) else {
            return Vec::new();
        };
        let range = node.range();
        let private = usage.private_at(range.start, range.end);
        let region = |capture: &str| capture_range(env, capture);
        let listed = |rules: [&Option<AstMatcher>; 3], nodes: &[Option<NodeRef>; 3]| {
            rules
                .iter()
                .zip(nodes)
                .all(|(rule, node)| rule.is_none() || node.is_some())
        };
        let own = [&self.site, &self.value, &self.annotation];
        if let Some((member, index)) = private {
            let declaration = &member.declarations[index];
            let nodes = [
                Some(declaration.site),
                declaration.value,
                declaration.annotation,
            ];
            let pre = match self.filter.private_verdict(&member, index, &region) {
                Some(true) if listed(own, &nodes) => PASSES,
                None => OPEN,
                _ => return Vec::new(),
            };
            return vec![Candidate {
                ordinal: PRIVATE,
                pre,
                nodes,
                shadow: PASSES,
                previous: [None; 3],
            }];
        }
        let shadowed_only = self.redefines_shadowed();
        let mut candidates = Vec::new();
        for (ordinal, id) in usage
            .named_at(range.start, range.end, node.kind_id())
            .enumerate()
        {
            let ordinal = ordinal as i64;
            let open = Candidate {
                ordinal,
                pre: OPEN,
                nodes: [None; 3],
                shadow: PASSES,
                previous: [None; 3],
            };
            match self.filter.verdict(&table, Some(&usage), id, &region) {
                Some(true) => {}
                Some(false) => continue,
                None => {
                    candidates.push(open);
                    continue;
                }
            }
            let shadowed = match self.redefines.as_deref().filter(|_| shadowed_only) {
                None => None,
                Some(redefines) => match Self::shadowed(redefines, &table, &usage, id, &region) {
                    Some(Some(shadowed)) => Some((redefines, shadowed)),
                    Some(None) => continue,
                    None => {
                        candidates.push(open);
                        continue;
                    }
                },
            };
            let binding = table.binding_of(id);
            let nodes = [
                binding.site.node,
                binding.site.value,
                binding.site.annotation,
            ];
            if !listed(own, &nodes) {
                continue;
            }
            let (shadow, previous) = match shadowed {
                None => (PASSES, [None; 3]),
                Some((redefines, shadowed)) => {
                    let previous = table.binding_of(shadowed);
                    let previous = [
                        previous.site.node,
                        previous.site.value,
                        previous.site.annotation,
                    ];
                    let rules = [&redefines.site, &redefines.value, &redefines.annotation];
                    let shadow = if listed(rules, &previous) {
                        PASSES
                    } else {
                        FAILS
                    };
                    (shadow, previous)
                }
            };
            candidates.push(Candidate {
                ordinal,
                pre: PASSES,
                nodes,
                shadow,
                previous,
            });
        }
        candidates
    }

    /// The occurrences of candidate `ordinal`'s variable (`Binds::candidates`).
    pub(crate) fn occurrences<D: Doc>(&self, node: &Node<'_, D>, ordinal: i64) -> Vec<NodeRef> {
        let Some((_, usage)) = self.tables(node) else {
            return Vec::new();
        };
        let range = node.range();
        if let Some((member, _)) = usage.private_at(range.start, range.end) {
            return if ordinal == PRIVATE {
                member.occurrences.clone()
            } else {
                Vec::new()
            };
        }
        usage
            .named_at(range.start, range.end, node.kind_id())
            .nth(usize::try_from(ordinal).unwrap_or(usize::MAX))
            .map(|id| usage.occurrences(id).to_vec())
            .unwrap_or_default()
    }

    /// The bindings `redefines` reads beside candidate `ordinal`, in the matcher's order, each
    /// with its verdict before its sub-rules (`redefines_others`, or `match_private`'s loop): a
    /// failing one is left out unless `every` reads it, an open one ends a private member's
    /// search, and an `OPEN` one ends the list where others may exist the table does not know.
    pub(crate) fn others_of<'tree, D: Doc>(
        &self,
        node: &Node<'tree, D>,
        env: &MetaVarEnv<'tree, D>,
        ordinal: i64,
    ) -> Vec<Other> {
        let (Some(redefines), Some((table, usage))) =
            (self.redefines.as_deref(), self.tables(node))
        else {
            return Vec::new();
        };
        let range = node.range();
        let private = usage.private_at(range.start, range.end);
        let region = |capture: &str| capture_range(env, capture);
        let rules = [&redefines.site, &redefines.value, &redefines.annotation];
        let complete = |nodes: &[Option<NodeRef>; 3]| {
            rules
                .iter()
                .zip(nodes)
                .all(|(rule, node)| rule.is_none() || node.is_some())
        };
        let failing = || {
            if redefines.every {
                vec![Other {
                    pre: FAILS,
                    nodes: [None; 3],
                }]
            } else {
                Vec::new()
            }
        };
        let mut others = Vec::new();
        // A passing one, or for `every` a failing one, in order.
        let push = |pre: u8, nodes: [Option<NodeRef>; 3], others: &mut Vec<Other>| {
            let pre = if pre == PASSES && !complete(&nodes) {
                FAILS
            } else {
                pre
            };
            if pre != FAILS || redefines.every {
                others.push(Other { pre, nodes });
            }
        };
        if let Some((member, index)) = private {
            if ordinal != PRIVATE || redefines.same_scope == Some(false) {
                return failing();
            }
            for other in 0..member.declarations.len() {
                if (redefines.every && other == index) || (!redefines.every && other >= index) {
                    continue;
                }
                let declaration = &member.declarations[other];
                let nodes = [
                    Some(declaration.site),
                    declaration.value,
                    declaration.annotation,
                ];
                match redefines.filter.private_verdict(&member, other, &region) {
                    Some(true) => push(PASSES, nodes, &mut others),
                    Some(false) => push(FAILS, nodes, &mut others),
                    // The matcher's answer is unknown from here on.
                    None => {
                        others.push(Other {
                            pre: OPEN,
                            nodes: [None; 3],
                        });
                        break;
                    }
                }
            }
            return others;
        }
        if self.redefines_shadowed() {
            return Vec::new();
        }
        let Some(id) = usage
            .named_at(range.start, range.end, node.kind_id())
            .nth(usize::try_from(ordinal).unwrap_or(usize::MAX))
        else {
            return Vec::new();
        };
        let Some((list, incomplete)) = Self::others(redefines, &table, id) else {
            return failing();
        };
        for (other, admitted) in list {
            let previous = table.binding_of(other);
            let nodes = [
                previous.site.node,
                previous.site.value,
                previous.site.annotation,
            ];
            let pre = if admitted {
                match redefines.filter.verdict(&table, Some(&usage), other, &region) {
                    Some(true) => PASSES,
                    Some(false) => FAILS,
                    None => OPEN,
                }
            } else {
                FAILS
            };
            push(pre, nodes, &mut others);
        }
        if incomplete {
            others.push(Other {
                pre: OPEN,
                nodes: [None; 3],
            });
        }
        others
    }
}

/// Whether `family` reads the file's flow (`ScopeRules::FLOW`).
fn flow(family: Family) -> bool {
    with_adapter!(family, A => <<A as Adapter>::Scope as ScopeRules>::FLOW)
}
