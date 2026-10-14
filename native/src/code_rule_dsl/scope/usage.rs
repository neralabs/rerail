//! Usedness: which reads resolve to each binding. Python follows Ruff 0.16.9's semantic model,
//! with what a binding inherits from the one it shadows; TypeScript and JavaScript follow ESLint's
//! no-unused-vars (`typescript`). Built once per table, on first use by a `binds` test or a usage
//! filter, and charged to each clause that reads it (`maxVisitedNodes`).

mod ecma;
mod php;

use super::{
    Binding, BuildError, DEADLINE_INTERVAL, Form, ImportKind, NodeRef, ScopeKind, ScopeTable,
};
use crate::code_rule_dsl::identifier::python_identifier;
use crate::code_rule_dsl::language::adapter::{Adapter, ScopeRules, with_adapter};
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt};
use std::borrow::Cow;
use std::collections::BTreeSet;

/// The forms whose usedness Ruff's `locals()`-aware checks read (F841's `Assignment`,
/// `NamedExprAssignment` and `WithItemVar`; the unused-argument rules' `Argument`): in a function
/// that calls `locals()`, any of them may be read unseen.
const LOCALS_FORMS: &[Form] = &[
    Form::Assignment,
    Form::Annotated,
    Form::Augmented,
    Form::Named,
    Form::With,
    Form::Match,
    Form::TypeAlias,
    Form::Parameter,
];

/// What the usage index knows about one binding. `None` is unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Facts {
    /// Reads that resolve to the binding itself: loads, `del`, `nonlocal`, `__all__` entries.
    pub(crate) reads: u32,
    /// Those of them made from another scope.
    pub(crate) nested: u32,
    /// Some read resolves to the binding, or to the same-scope binding it shadows while it
    /// inherits that one's reads (Ruff copies them, except from a deletion or an unbound
    /// exception name).
    pub(crate) used: Option<bool>,
    /// Some such read is made from another scope.
    pub(crate) nested_use: Option<bool>,
    /// Python: an explicit re-export (`import a as a`), or a module binding a static `__all__`
    /// names. TypeScript: a definition stands in an `export` statement.
    pub(crate) exported: Option<bool>,
    /// Some reference resolves to it, writes included (TypeScript: eslint-scope's `references`,
    /// the writes a definition's initializer and defaults make included); nothing inherited.
    pub(crate) referenced: Option<bool>,
    /// TypeScript: where ESLint reports the variable, by byte range: its last write from its own
    /// function scope, else its first definition's name.
    pub(crate) anchor: Option<(u32, u32)>,
    /// TypeScript: how many references eslint-scope gives the variable (reads, writes, and the
    /// writes at its definitions).
    pub(crate) references: u32,
}

/// A binding of the table: its scope, name and index among that scope's bindings of the name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BindingId<'t> {
    pub(crate) scope: u32,
    pub(crate) name: &'t str,
    pub(crate) index: usize,
}

/// A private name of one class body, ESLint 10.11.0's no-unused-private-class-members member: the
/// declarations of the name (a getter and a setter are one member) and what the body's references
/// to it do. No scope table holds it.
#[derive(Debug, Default)]
pub(crate) struct PrivateMember {
    pub(crate) declarations: Vec<PrivateDeclaration>,
    /// Some reference in the body reads it: any reference of an accessor; of a field or method,
    /// one that is not only written (`this.#a = 1`, a statement-level `this.#a += 1` or
    /// `this.#a++`, a `for ... in/of` head or a destructuring target).
    pub(crate) used: bool,
    /// Some reference in the body names it.
    pub(crate) referenced: bool,
    /// Its declaration names and references, in document order.
    pub(crate) occurrences: Vec<NodeRef>,
}

/// One declaration of a private name: a field, method or accessor of the class body.
#[derive(Debug)]
pub(crate) struct PrivateDeclaration {
    pub(crate) name: NodeRef,
    pub(crate) form: super::Form,
    /// The field or method definition.
    pub(crate) site: NodeRef,
    /// A field's initializer.
    pub(crate) value: Option<NodeRef>,
    /// A field's type, without its `:`.
    pub(crate) annotation: Option<NodeRef>,
}

/// A binding's name node, for finding the binding from the node.
struct NameSite {
    start: u32,
    end: u32,
    kind: u16,
    scope: u32,
    name: Box<str>,
    index: usize,
}

pub(crate) struct Usage {
    /// Per scope, per name, aligned with the scope's bindings of the name.
    facts: Vec<HashMap<Box<str>, Vec<Facts>>>,
    /// Sorted by start byte.
    sites: Vec<NameSite>,
    /// The string entries of the module's `__all__` definitions and the names they export, by
    /// start byte.
    exports: Vec<(u32, u32, Box<str>)>,
    /// TypeScript: the names `/* exported */` comments list.
    pub(crate) comment_exports: BTreeSet<Box<str>>,
    /// TypeScript and PHP: each variable's definition names and references, in document order.
    occurrences: HashMap<(u32, Box<str>), Vec<NodeRef>>,
    /// TypeScript: the private names of every class body.
    privates: Vec<PrivateMember>,
    /// Each private declaration's name node, with its member and index, by start byte.
    private_names: Vec<(NodeRef, u32, u32)>,
    /// The start of each reference that is its function's first read of the binding a later
    /// binding of the function shadows (Ruff's F823), sorted, and whether that binding is the
    /// builtin of the name, which holds only where the target version has one.
    before_local: Vec<(usize, bool)>,
    /// Steps the build took: what a clause pays for the index.
    pub(crate) cost: usize,
}

impl Usage {
    pub(crate) fn facts(&self, id: BindingId<'_>) -> Facts {
        self.facts[id.scope as usize]
            .get(id.name)
            .and_then(|facts| facts.get(id.index))
            .copied()
            .unwrap_or_default()
    }

    /// TypeScript and PHP: the definition names and references of `id`'s variable.
    pub(crate) fn occurrences(&self, id: BindingId<'_>) -> &[NodeRef] {
        self.occurrences
            .get(&(id.scope, Box::from(id.name)))
            .map_or(&[], Vec::as_slice)
    }

    /// The private member a declaration name node spanning exactly this range declares, with the
    /// declaration's index among the member's.
    pub(crate) fn private_at(&self, start: usize, end: usize) -> Option<(&PrivateMember, usize)> {
        let index = self
            .private_names
            .binary_search_by_key(&(start, end), |(name, _, _)| {
                (name.start as usize, name.end as usize)
            })
            .ok()?;
        let (_, member, declaration) = self.private_names[index];
        Some((&self.privates[member as usize], declaration as usize))
    }

    /// Whether the reference starting at `start` is its function's first read of the binding that
    /// the function's own (later) binding of the name shadows: `Some(true)` where that binding is
    /// the name's builtin, so only if the target version has one.
    pub(crate) fn before_local(&self, start: usize) -> Option<bool> {
        let index = self
            .before_local
            .binary_search_by_key(&start, |(start, _)| *start)
            .ok()?;
        Some(self.before_local[index].1)
    }

    /// The name a module `__all__` entry exports, if the string spanning exactly this range is one.
    pub(crate) fn export_at(&self, start: usize, end: usize) -> Option<&str> {
        let index = self
            .exports
            .binary_search_by_key(&(start, end), |(entry_start, entry_end, _)| {
                (*entry_start as usize, *entry_end as usize)
            })
            .ok()?;
        Some(&self.exports[index].2)
    }

    /// The bindings whose name node is exactly this node.
    pub(crate) fn named_at(
        &self,
        start: usize,
        end: usize,
        kind: u16,
    ) -> impl Iterator<Item = BindingId<'_>> {
        let first = self
            .sites
            .partition_point(|site| (site.start as usize) < start);
        self.sites[first..]
            .iter()
            .take_while(move |site| site.start as usize == start)
            .filter(move |site| site.end as usize == end && site.kind == kind)
            .map(|site| BindingId {
                scope: site.scope,
                name: &site.name,
                index: site.index,
            })
    }

    /// Builds the index of a table, whose file's root is `root`, in at most `cap` steps.
    pub(crate) fn build<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        cap: usize,
        deadline: &dyn Fn() -> bool,
    ) -> Result<Self, BuildError> {
        let mut steps = Steps {
            cost: 0,
            cap,
            deadline,
        };
        with_adapter!(table.family, A => <<A as Adapter>::Scope as ScopeRules>::usage(table, root, &mut steps))
    }

    /// TypeScript's and JavaScript's index: ESLint's no-unused-vars (`ecma`).
    pub(crate) fn ecma<D: Doc>(
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

    /// PHP's index, from its flow (`php`).
    pub(crate) fn php(table: &ScopeTable, steps: &mut Steps<'_>) -> Result<Self, BuildError> {
        let facts = php::build(table, steps)?;
        let occurrences = php::occurrences(table, steps)?;
        let sites = name_sites(table, steps)?;
        Ok(Self {
            facts,
            sites,
            exports: Vec::new(),
            comment_exports: BTreeSet::new(),
            occurrences,
            privates: Vec::new(),
            private_names: Vec::new(),
            before_local: Vec::new(),
            cost: steps.cost,
        })
    }

    /// SQL's index, from the file's query blocks (`sql::SqlScopes::usage`): a binding is used,
    /// and referenced, where AL05's walk finds a reference to it. SQL exports nothing.
    pub(crate) fn sql(table: &ScopeTable, steps: &mut Steps<'_>) -> Result<Self, BuildError> {
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

    /// Python's index: Ruff's semantic model, with what a binding inherits from the one it
    /// shadows.
    pub(crate) fn python<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Self, BuildError> {
        let text = root.text();
        let base = root.range().start;
        let mut facts: Vec<HashMap<Box<str>, Vec<Facts>>> = table
            .scopes
            .iter()
            .map(|scope| {
                scope
                    .bindings
                    .iter()
                    .map(|(name, bindings)| (name.clone(), vec![Facts::default(); bindings.len()]))
                    .collect()
            })
            .collect();
        // Ruff honours `locals()` only in the function that calls it, not in a lambda or a
        // comprehension, and reads no other builtin (`vars`, `eval`, `exec`, `globals`) so.
        let mut uses_locals = vec![false; table.scopes.len()];
        for &scope in &table.locals_scopes {
            uses_locals[scope as usize] = table.scopes[scope as usize].kind == ScopeKind::Function;
        }
        // Per function and name, its first read (by where it is read) of the binding its own
        // binding of the name shadows, and whether that is the builtin.
        let mut first_reads: HashMap<(u32, Cow<'_, str>), (usize, usize, bool)> = HashMap::new();
        // Where each read of a binding that an `except` name later shadows is made: `None` for one
        // Ruff makes after the whole scope (from a function body), and whether it is nested.
        #[allow(clippy::type_complexity)]
        let mut positions: HashMap<(u32, Cow<'_, str>, usize), Vec<(Option<usize>, bool)>> =
            HashMap::new();
        for reference in &table.references {
            steps.take()?;
            let Some(name) = table.reference_name(reference, &text, base) else {
                continue;
            };
            let reads = table.reads(reference, &name);
            let before = match table.shadowed_by_local(reference, &name) {
                Some(Shadowed::Binding(shadowed)) => reads.contains(&shadowed).then_some(false),
                // A read that resolves to nothing reads the builtin, if the name is one.
                Some(Shadowed::Builtin) => reads.is_empty().then_some(true),
                None => None,
            };
            if let Some(builtin) = before {
                let key = (reference.scope, name.clone());
                let at = (reference.at, reference.start, builtin);
                first_reads
                    .entry(key)
                    .and_modify(|first| *first = (*first).min(at))
                    .or_insert(at);
            }
            for (scope, index) in reads {
                let read = &mut facts[scope as usize]
                    .get_mut(name.as_ref())
                    .expect("a resolved name has facts")[index];
                read.reads += 1;
                read.nested += u32::from(reference.scope != scope);
                // An `except` name inherits only the reads made before it (below); a read after
                // its handler may resolve to the binding before it.
                let chain = &table.scopes[scope as usize].bindings[name.as_ref()];
                if chain[index + 1..]
                    .iter()
                    .any(|binding| binding.until.is_some())
                {
                    let late = reference.deferred || table.crosses_function(reference.scope, scope);
                    positions
                        .entry((scope, name.clone(), index))
                        .or_default()
                        .push(((!late).then_some(reference.at), reference.scope != scope));
                }
            }
            if let Some((scope, submodule, index)) = table.submodule_alias(reference, &name) {
                let read = &mut facts[scope as usize]
                    .get_mut(submodule)
                    .expect("a submodule import has facts")[index];
                read.reads += 1;
                read.nested += u32::from(reference.scope != scope);
            }
        }
        let exports = dunder_all(table, root, steps)?;
        for (_, _, name) in &exports.entries {
            if let Some(bindings) = table.scopes[0].bindings.get(name.as_str())
                && let Some(last) = facts[0]
                    .get_mut(name.as_str())
                    .and_then(|facts| facts.get_mut(bindings.len() - 1))
            {
                last.reads += 1;
            }
        }
        for (scope_index, scope) in table.scopes.iter().enumerate() {
            let module = scope_index == 0;
            for (name, bindings) in &scope.bindings {
                steps.take()?;
                let chain = facts[scope_index]
                    .get_mut(name)
                    .expect("every binding has facts");
                let unknown = module && !exports.complete;
                let mut previous: Option<(bool, bool)> = None;
                let mut previous_inherited = (false, false);
                for (index, binding) in bindings.iter().enumerate() {
                    let inherited = if index > 0 && inherits(&bindings[index - 1], binding) {
                        if binding.until.is_some() {
                            // Ruff copies the references when it adds the binding: an `except`
                            // name gets the reads made before its handler, not those after it,
                            // which resolve to the binding before it.
                            let early = positions
                                .get(&(scope_index as u32, Cow::Borrowed(name.as_ref()), index - 1))
                                .into_iter()
                                .flatten()
                                .filter(|(at, _)| at.is_some_and(|at| at < binding.offset));
                            let (mut used, mut nested) = previous_inherited;
                            for (_, read_nested) in early {
                                used = true;
                                nested |= *read_nested;
                            }
                            Some((used, nested))
                        } else {
                            previous
                        }
                    } else {
                        None
                    };
                    previous_inherited = inherited.unwrap_or_default();
                    let unknown = unknown
                        || (uses_locals[scope_index] && LOCALS_FORMS.contains(&binding.site.form));
                    let own = &mut chain[index];
                    let used = own.reads > 0 || inherited.is_some_and(|(used, _)| used);
                    let nested = own.nested > 0 || inherited.is_some_and(|(_, nested)| nested);
                    own.used = known(used, unknown);
                    own.nested_use = known(nested, unknown);
                    own.referenced = known(own.reads > 0, unknown);
                    own.exported = if binding.explicit_export
                        || (module && exports.names.contains(name.as_ref()))
                    {
                        Some(true)
                    } else if module && !exports.complete {
                        None
                    } else {
                        Some(false)
                    };
                    previous = Some((used, nested));
                }
            }
        }
        let sites = name_sites(table, steps)?;
        let mut entries = exports
            .entries
            .into_iter()
            .map(|(start, end, name)| (start as u32, end as u32, name.into_boxed_str()))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(start, end, _)| (*start, *end));
        let mut before_local = first_reads
            .into_values()
            .map(|(_, start, builtin)| (start, builtin))
            .collect::<Vec<_>>();
        before_local.sort_unstable();
        Ok(Self {
            facts,
            sites,
            exports: entries,
            comment_exports: BTreeSet::new(),
            occurrences: HashMap::new(),
            privates: Vec::new(),
            private_names: Vec::new(),
            before_local,
            cost: steps.cost,
        })
    }
}

/// What a function's own binding of a name shadows in an enclosing scope (`shadowed_by_local`).
enum Shadowed {
    /// An enclosing function's or the module's binding, by scope and index.
    Binding((u32, usize)),
    /// No binding: the name's builtin, if the target version has one.
    Builtin,
}

/// Every binding's name node, sorted by position.
fn name_sites(table: &ScopeTable, steps: &mut Steps<'_>) -> Result<Vec<NameSite>, BuildError> {
    let mut sites = Vec::new();
    for (scope_index, scope) in table.scopes.iter().enumerate() {
        for (name, bindings) in &scope.bindings {
            for (index, binding) in bindings.iter().enumerate() {
                if let Some(node) = binding.name {
                    steps.take()?;
                    sites.push(NameSite {
                        start: node.start,
                        end: node.end,
                        kind: node.kind,
                        scope: scope_index as u32,
                        name: name.clone(),
                        index,
                    });
                }
            }
        }
    }
    sites.sort_by_key(|site| (site.start, site.end));
    Ok(sites)
}

/// `true` is known; `false` is unknown where something may read the name unseen.
fn known(value: bool, unknown: bool) -> Option<bool> {
    (value || !unknown).then_some(value)
}

/// Whether `binding` inherits the reads of `previous`, the same-scope binding it shadows: Ruff
/// copies them unless that one is a deletion or an `except` name its handler has unbound. A
/// nested function's `nonlocal` bindings, which Ruff keeps in the nested scope, inherit only from
/// each other.
fn inherits(previous: &Binding, binding: &Binding) -> bool {
    previous.deferred == binding.deferred
        && !previous.is_deletion()
        && previous.until.is_none_or(|until| binding.offset < until)
}

/// The steps an index's build spends, against its cap and the evaluation deadline.
pub(crate) struct Steps<'d> {
    cost: usize,
    cap: usize,
    deadline: &'d dyn Fn() -> bool,
}

impl Steps<'_> {
    fn take(&mut self) -> Result<(), BuildError> {
        self.cost += 1;
        if self.cost > self.cap {
            return Err(BuildError::Budget);
        }
        if self.cost.is_multiple_of(DEADLINE_INTERVAL) && (self.deadline)() {
            return Err(BuildError::Deadline);
        }
        Ok(())
    }
}

/// The names the module's `__all__` definitions list, and whether every definition is static.
struct Exports {
    names: BTreeSet<String>,
    /// Each string entry: its range and the name.
    entries: Vec<(usize, usize, String)>,
    complete: bool,
}

impl Exports {
    fn add<D: Doc>(&mut self, element: &Node<'_, D>) {
        match string_name(element) {
            Some(name) => {
                let range = element.range();
                self.names.insert(name.clone());
                self.entries.push((range.start, range.end, name));
            }
            None => self.complete = false,
        }
    }
}

/// Ruff's `extract_dunder_all_names` over each module-level `__all__ =`, `+=` or `: T =`: string
/// entries of lists and tuples, joined by `+`, and `list(...)`/`tuple(...)` of a display. Anything
/// else, a non-string entry included, makes the exports incomplete (Ruff ignores it).
fn dunder_all<D: Doc>(
    table: &ScopeTable,
    root: &Node<'_, D>,
    steps: &mut Steps<'_>,
) -> Result<Exports, BuildError> {
    let mut exports = Exports {
        names: BTreeSet::new(),
        entries: Vec::new(),
        complete: true,
    };
    for statement in &table.dunder_all {
        steps.take()?;
        let Some(statement) = statement.find(root.clone()) else {
            continue;
        };
        let mut value = statement.field("right");
        while let Some(inner) = value.clone().filter(|value| value.kind() == "assignment") {
            value = inner.field("right");
        }
        match value {
            Some(value) => extract_value(table, &value, &mut exports, steps)?,
            None => exports.complete = false,
        }
    }
    Ok(exports)
}

/// An `__all__` statement's value: a chain of binary operations, any operator, reads the right
/// side of each and the leftmost operand (Ruff's loop in `extract_dunder_all_names`); a binary
/// operation on a right side is not read.
fn extract_value<D: Doc>(
    table: &ScopeTable,
    value: &Node<'_, D>,
    exports: &mut Exports,
    steps: &mut Steps<'_>,
) -> Result<(), BuildError> {
    let mut current = unparenthesized(value);
    while current.kind() == "binary_operator" {
        steps.take()?;
        let (Some(left), Some(right)) = (current.field("left"), current.field("right")) else {
            exports.complete = false;
            return Ok(());
        };
        extract(table, &unparenthesized(&right), exports, steps)?;
        current = unparenthesized(&left);
    }
    extract(table, &current, exports, steps)
}

/// The expression inside any parentheses around `node` (Ruff's AST has none).
fn unparenthesized<'r, D: Doc>(node: &Node<'r, D>) -> Node<'r, D> {
    let mut node = node.clone();
    while node.kind() == "parenthesized_expression" {
        let mut inner = node
            .children()
            .filter(|child| child.is_named() && child.kind() != "comment");
        let (Some(expression), None) = (inner.next(), inner.next()) else {
            break;
        };
        drop(inner);
        node = expression;
    }
    node
}

/// Whether a call's callee is the builtin `list` or `tuple`, as Ruff's `resolve_builtin_symbol`
/// reads it through `map_subscript`: the name, unshadowed where the call is (or imported from
/// `builtins`), or `builtins.list` through an import of `builtins`; `list[str]` too.
fn builtin_sequence<D: Doc>(table: &ScopeTable, function: &Node<'_, D>) -> bool {
    let mut callee = unparenthesized(function);
    if callee.kind() == "subscript"
        && let Some(value) = callee.field("value")
    {
        callee = unparenthesized(&value);
    }
    let sequence = |name: &str| matches!(name, "list" | "tuple");
    let imports = |binding: &Binding, qualified: &dyn Fn(&str) -> bool| match &binding.import {
        Some(super::Import::Qualified(name)) => qualified(name),
        _ => false,
    };
    match callee.kind().as_ref() {
        "identifier" => {
            let name = python_identifier(&callee);
            let Some(reference) = table.reference(&callee.range()) else {
                return false;
            };
            match table.resolve(reference, &name) {
                None => sequence(&name),
                Some(binding) if !binding.shadows() => sequence(&name),
                Some(binding) => imports(binding, &|qualified| {
                    qualified.strip_prefix("builtins.").is_some_and(sequence)
                }),
            }
        }
        "attribute" => {
            let (Some(object), Some(attribute)) =
                (callee.field("object"), callee.field("attribute"))
            else {
                return false;
            };
            let object = unparenthesized(&object);
            object.kind() == "identifier"
                && sequence(&python_identifier(&attribute))
                && table.reference(&object.range()).is_some_and(|reference| {
                    table
                        .resolve(reference, &python_identifier(&object))
                        .is_some_and(|binding| {
                            imports(binding, &|qualified| qualified == "builtins")
                        })
                })
        }
        _ => false,
    }
}

fn extract<D: Doc>(
    table: &ScopeTable,
    node: &Node<'_, D>,
    exports: &mut Exports,
    steps: &mut Steps<'_>,
) -> Result<(), BuildError> {
    steps.take()?;
    let named = || {
        node.children()
            .filter(|child| child.is_named() && child.kind() != "comment")
    };
    match node.kind().as_ref() {
        "parenthesized_expression" => match named().collect::<Vec<_>>().as_slice() {
            [inner] => extract(table, inner, exports, steps)?,
            _ => exports.complete = false,
        },
        "named_expression" => match node.field("value") {
            Some(value) => extract(table, &value, exports, steps)?,
            None => exports.complete = false,
        },
        "list" | "tuple" => {
            for element in named() {
                steps.take()?;
                exports.add(&element);
            }
        }
        // Another `__all__` adds nothing this module binds.
        "identifier" if python_identifier(node) == "__all__" => {}
        // A value that holds no names (Ruff's `INVALID_FORMAT`, which lists none either).
        "none" | "true" | "false" | "integer" | "float" | "ellipsis" => {}
        "unary_operator"
            if node.field("argument").is_some_and(|argument| {
                matches!(argument.kind().as_ref(), "integer" | "float")
            }) => {}
        "attribute"
            if node
                .field("attribute")
                .is_some_and(|attribute| python_identifier(&attribute) == "__all__") => {}
        "call"
            if node
                .field("function")
                .is_some_and(|function| builtin_sequence(table, &function)) =>
        {
            let arguments = node
                .field("arguments")
                .map(|arguments| {
                    arguments
                        .children()
                        .filter(|child| child.is_named() && child.kind() != "comment")
                        .map(|argument| unparenthesized(&argument))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            match arguments.as_slice() {
                [] => {}
                [display] if matches!(display.kind().as_ref(), "list" | "tuple" | "set") => {
                    let set = display.kind() == "set";
                    if set {
                        for element in display
                            .children()
                            .filter(|child| child.is_named() && child.kind() != "comment")
                        {
                            steps.take()?;
                            exports.add(&element);
                        }
                    } else {
                        extract(table, display, exports, steps)?;
                    }
                }
                _ => exports.complete = false,
            }
        }
        _ => exports.complete = false,
    }
    Ok(())
}

/// The name a string entry spells: Ruff's `ExprStringLiteral` value, escapes decoded and an
/// implicit concatenation joined; none for an f-, t- or bytes literal.
fn string_name<D: Doc>(node: &Node<'_, D>) -> Option<String> {
    crate::code_rule_dsl::value::python::string_literal(node)
}

impl ScopeTable {
    /// The binding `id` shadows, Ruff's `shadowed_bindings` pair, and whether it is of the same
    /// scope: the binding before it in its scope, or, for the first binding of its name in its
    /// scope, the latest one of the nearest enclosing function or module scope that binds the name
    /// when `id` is made (a function body is read after the scopes around it). A nested function's
    /// `nonlocal` binding shadows none (Ruff keeps it after the `nonlocal` declaration).
    pub(crate) fn shadowed<'t>(&'t self, id: BindingId<'t>) -> Option<(BindingId<'t>, bool)> {
        let chain = self.chain(id);
        let binding = &chain[id.index];
        if id.index > 0 {
            let previous = &chain[id.index - 1];
            return (previous.deferred == binding.deferred).then_some((
                BindingId {
                    index: id.index - 1,
                    ..id
                },
                true,
            ));
        }
        if binding.deferred {
            return None;
        }
        let scope = &self.scopes[id.scope as usize];
        let mut crossed = matches!(
            scope.kind,
            super::ScopeKind::Function | super::ScopeKind::Lambda
        );
        let mut current = scope.parent;
        while let Some(index) = current {
            let ancestor = &self.scopes[index as usize];
            if matches!(
                ancestor.kind,
                super::ScopeKind::Function | super::ScopeKind::Module
            ) && let Some(found) = Self::visible(ancestor, id.name, crossed, binding.offset)
            {
                return Some((
                    BindingId {
                        scope: index,
                        name: id.name,
                        index: found,
                    },
                    false,
                ));
            }
            crossed |= matches!(
                ancestor.kind,
                super::ScopeKind::Function | super::ScopeKind::Lambda
            );
            current = ancestor.parent;
        }
        None
    }

    /// Whether a function or lambda body lies between `inner` and its ancestor `outer`: Ruff visits
    /// what `inner` reads after `outer`'s whole body.
    fn crosses_function(&self, inner: u32, outer: u32) -> bool {
        let mut current = Some(inner);
        while let Some(index) = current.filter(|index| *index != outer) {
            let scope = &self.scopes[index as usize];
            if matches!(scope.kind, ScopeKind::Function | ScopeKind::Lambda) {
                return true;
            }
            current = scope.parent;
        }
        false
    }

    /// Ruff's `undefined_local` pair for a load in a function, outside the type scope of a
    /// definition inside it and not a forward reference the module resolves, which Ruff reads
    /// from the module scope (Ruff's `reference.scope_id() == scope_id`): where the function's latest own
    /// binding of the name is also its first, the binding of an enclosing function or the module
    /// that it shadows (Ruff's `shadowed_binding`, which `global`, `nonlocal` and `del` bindings
    /// never have), or with none such the builtin, which Ruff materializes in the module scope
    /// before it adds any binding of a builtin's name.
    fn shadowed_by_local(&self, reference: &super::Reference, name: &str) -> Option<Shadowed> {
        if reference.access != super::Access::Load
            || reference.type_scope
            || self.forward_module(reference, name).is_some()
        {
            return None;
        }
        let scope = &self.scopes[reference.scope as usize];
        if scope.kind != ScopeKind::Function || scope.nonlocals.contains(name) {
            return None;
        }
        let mut own = scope
            .bindings
            .get(name)?
            .iter()
            .enumerate()
            .filter(|(_, binding)| !binding.deferred);
        let (index, binding) = own.next()?;
        // An `except` name is two bindings in Ruff, the second shadowing only the first.
        if own.next().is_some()
            || binding.is_deletion()
            || binding.until.is_some()
            || binding.origin != super::Origin::Bound
        {
            return None;
        }
        match self.shadowed(BindingId {
            scope: reference.scope,
            name,
            index,
        }) {
            Some((shadowed, false)) => Some(Shadowed::Binding((shadowed.scope, shadowed.index))),
            Some((_, true)) => None,
            None => Some(Shadowed::Builtin),
        }
    }

    /// The qualified name a binding imports, as Ruff compares imports (`import a.b` imports
    /// `a.b`); relative imports by their level and text.
    pub(crate) fn imported(binding: &Binding) -> Option<String> {
        binding.import_kind?;
        if let Some(submodule) = &binding.submodule {
            return Some(submodule.to_string());
        }
        match binding.import.as_ref()? {
            super::Import::Qualified(name) => Some(name.to_string()),
            super::Import::Relative { level, rest } => {
                Some(format!("{}{rest}", ".".repeat(*level)))
            }
            super::Import::Definition => None,
        }
    }

    /// The bindings a Python reference reads: the one it resolves to and, for a read through a
    /// `global` declaration, that declaration too (Ruff marks both used).
    /// A load also reads each bare annotation its walk meets (Ruff's `resolve_load` marks them).
    pub(crate) fn reads(&self, reference: &super::Reference, name: &str) -> Vec<(u32, usize)> {
        if reference.access == super::Access::Del {
            return self
                .resolve_python_index(reference, name)
                .into_iter()
                .collect();
        }
        let walk = self.walk(reference, name, true);
        let mut reads = walk.annotations;
        if let Some((found, module)) = walk.found {
            reads.push(module.unwrap_or(found));
            if module.is_some_and(|module| module != found) {
                reads.push(found);
            }
        }
        reads
    }

    /// Ruff's `resolve_submodule`: a read of `pa` bound by `import pyarrow as pa` also uses the
    /// latest `import pyarrow.csv` of the same scope.
    fn submodule_alias<'t>(
        &'t self,
        reference: &super::Reference,
        name: &str,
    ) -> Option<(u32, &'t str, usize)> {
        let (scope, index) = self.resolve_python_index(reference, name)?;
        let binding = &self.scopes[scope as usize].bindings[name][index];
        if binding.import_kind != Some(ImportKind::Module) {
            return None;
        }
        let Some(super::Import::Qualified(qualified)) = &binding.import else {
            return None;
        };
        let segment = qualified.rsplit('.').next()?;
        if segment == name {
            return None;
        }
        let (key, bindings) = self.scopes[scope as usize]
            .bindings
            .get_key_value(segment)?;
        let crossed = reference.deferred || reference.scope != scope;
        let found = super::latest(bindings, |candidate| {
            if crossed {
                candidate.until.is_none()
            } else {
                !candidate.deferred && candidate.offset <= reference.at
            }
        })?;
        let submodule = &bindings[found];
        let module = |qualified: &str| qualified.split('.').next().map(str::to_owned);
        (submodule.import_kind == Some(ImportKind::Submodule)
            && submodule.submodule.as_deref().and_then(module) == module(qualified))
        .then_some((scope, key.as_ref(), found))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::scope::ANNOTATION_RANGE;

    #[test]
    fn before_local_keeps_synthetic_annotation_offsets() {
        let usage = Usage {
            facts: Vec::new(),
            sites: Vec::new(),
            exports: Vec::new(),
            comment_exports: BTreeSet::new(),
            occurrences: HashMap::new(),
            privates: Vec::new(),
            private_names: Vec::new(),
            before_local: vec![(ANNOTATION_RANGE, true)],
            cost: 0,
        };
        assert_eq!(usage.before_local(ANNOTATION_RANGE), Some(true));
    }
}
