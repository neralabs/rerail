//! Usedness: which reads resolve to each binding. Python follows Ruff 0.16.9's semantic model,
//! with what a binding inherits from the one it shadows; TypeScript and JavaScript follow ESLint's
//! no-unused-vars (`typescript`). Built once per table, on first use by a `binds` test or a usage
//! filter, and charged to each clause that reads it (`maxVisitedNodes`).

// cratify: `ecma` is rerail-lang-ecma's (it mounts it)
// cratify: `php` is rerail-lang-php's (it mounts it)
// cratify: `rust` is rerail-lang-rust's (it mounts it)

use super::{
    Binding, BuildError, DEADLINE_INTERVAL, Form, ImportKind, NodeRef, ScopeKind, ScopeTable,
};
use crate::code_rule_dsl::tree::Tree;
use crate::code_rule_dsl::identifier::python_identifier;
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt};
use std::borrow::Cow;
use std::collections::BTreeSet;
use crate::code_rule_dsl::language::adapter::pack as language_pack;

/// The forms whose usedness Ruff's `locals()`-aware checks read (F841's `Assignment`,
/// `NamedExprAssignment` and `WithItemVar`; the unused-argument rules' `Argument`): in a function
/// that calls `locals()`, any of them may be read unseen.
pub const LOCALS_FORMS: &[Form] = &[
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
pub struct Facts {
    /// Reads that resolve to the binding itself: loads, `del`, `nonlocal`, `__all__` entries.
    pub reads: u32,
    /// Those of them made from another scope.
    pub nested: u32,
    /// Some read resolves to the binding, or to the same-scope binding it shadows while it
    /// inherits that one's reads (Ruff copies them, except from a deletion or an unbound
    /// exception name).
    pub used: Option<bool>,
    /// Some such read is made from another scope.
    pub nested_use: Option<bool>,
    /// Python: an explicit re-export (`import a as a`), or a module binding a static `__all__`
    /// names. TypeScript: a definition stands in an `export` statement.
    pub exported: Option<bool>,
    /// Some reference resolves to it, writes included (TypeScript: eslint-scope's `references`,
    /// the writes a definition's initializer and defaults make included); nothing inherited.
    pub referenced: Option<bool>,
    /// TypeScript: where ESLint reports the variable, by byte range: its last write from its own
    /// function scope, else its first definition's name.
    pub anchor: Option<(u32, u32)>,
    /// TypeScript: how many references eslint-scope gives the variable (reads, writes, and the
    /// writes at its definitions).
    pub references: u32,
}

/// A binding of the table: its scope, name and index among that scope's bindings of the name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BindingId<'t> {
    pub scope: u32,
    pub name: &'t str,
    pub index: usize,
}

/// A private name of one class body, ESLint 10.11.0's no-unused-private-class-members member: the
/// declarations of the name (a getter and a setter are one member) and what the body's references
/// to it do. No scope table holds it.
#[derive(Debug, Default)]
pub struct PrivateMember {
    pub declarations: Vec<PrivateDeclaration>,
    /// Some reference in the body reads it: any reference of an accessor; of a field or method,
    /// one that is not only written (`this.#a = 1`, a statement-level `this.#a += 1` or
    /// `this.#a++`, a `for ... in/of` head or a destructuring target).
    pub used: bool,
    /// Some reference in the body names it.
    pub referenced: bool,
    /// Its declaration names and references, in document order.
    pub occurrences: Vec<NodeRef>,
}

/// One declaration of a private name: a field, method or accessor of the class body.
#[derive(Debug)]
pub struct PrivateDeclaration {
    pub name: NodeRef,
    pub form: super::Form,
    /// The field or method definition.
    pub site: NodeRef,
    /// A field's initializer.
    pub value: Option<NodeRef>,
    /// A field's type, without its `:`.
    pub annotation: Option<NodeRef>,
}

/// A binding's name node, for finding the binding from the node.
pub struct NameSite {
    start: u32,
    end: u32,
    kind: u16,
    scope: u32,
    name: Box<str>,
    index: usize,
}

pub struct Usage {
    /// Per scope, per name, aligned with the scope's bindings of the name.
    pub facts: Vec<HashMap<Box<str>, Vec<Facts>>>,
    /// Sorted by start byte.
    pub sites: Vec<NameSite>,
    /// The string entries of the module's `__all__` definitions and the names they export, by
    /// start byte.
    pub exports: Vec<(u32, u32, Box<str>)>,
    /// TypeScript: the names `/* exported */` comments list.
    pub comment_exports: BTreeSet<Box<str>>,
    /// TypeScript and PHP: each variable's definition names and references, in document order.
    pub occurrences: HashMap<(u32, Box<str>), Vec<NodeRef>>,
    /// TypeScript: the private names of every class body.
    pub privates: Vec<PrivateMember>,
    /// Each private declaration's name node, with its member and index, by start byte.
    pub private_names: Vec<(NodeRef, u32, u32)>,
    /// The start of each reference that is its function's first read of the binding a later
    /// binding of the function shadows (Ruff's F823), sorted, and whether that binding is the
    /// builtin of the name, which holds only where the target version has one.
    pub before_local: Vec<(usize, bool)>,
    /// Steps the build took: what a clause pays for the index.
    pub cost: usize,
}

impl Usage {
    /// Declaration metadata does not require the language's read/usedness analysis.
    pub(crate) fn declarations(
        table: &ScopeTable,
        cap: usize,
        deadline: &dyn Fn() -> bool,
    ) -> Result<Self, BuildError> {
        let mut steps = Steps {
            cost: 0,
            cap,
            deadline,
        };
        let sites = name_sites(table, &mut steps)?;
        Ok(Self {
            sites,
            facts: Vec::new(),
            exports: Vec::new(),
            comment_exports: BTreeSet::new(),
            occurrences: HashMap::new(),
            privates: Vec::new(),
            private_names: Vec::new(),
            before_local: Vec::new(),
            cost: steps.cost,
        })
    }
    pub fn facts(&self, id: BindingId<'_>) -> Facts {
        self.facts[id.scope as usize]
            .get(id.name)
            .and_then(|facts| facts.get(id.index))
            .copied()
            .unwrap_or_default()
    }

    /// TypeScript and PHP: the definition names and references of `id`'s variable.
    pub fn occurrences(&self, id: BindingId<'_>) -> &[NodeRef] {
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
    pub fn before_local(&self, start: usize) -> Option<bool> {
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
    pub fn build<D: Doc>(
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
        { let pack = language_pack::pack(table.family); language_pack::scope_usage(pack, table, root, &mut steps) }
    }

    /// The index of a file whose family's scope the engine does not read yet (registry
    /// `not-supported-yet`): no binding carries a fact and no name is a site.
    pub fn unread(table: &ScopeTable, steps: &Steps<'_>) -> Self {
        Self {
            facts: table.scopes.iter().map(|_| HashMap::new()).collect(),
            sites: Vec::new(),
            exports: Vec::new(),
            comment_exports: BTreeSet::new(),
            occurrences: HashMap::new(),
            privates: Vec::new(),
            private_names: Vec::new(),
            before_local: Vec::new(),
            cost: steps.cost,
        }
    }
}

/// What a function's own binding of a name shadows in an enclosing scope (`shadowed_by_local`).
pub enum Shadowed {
    /// An enclosing function's or the module's binding, by scope and index.
    Binding((u32, usize)),
    /// No binding: the name's builtin, if the target version has one.
    Builtin,
}

/// Every binding's name node, sorted by position.
pub fn name_sites(table: &ScopeTable, steps: &mut Steps<'_>) -> Result<Vec<NameSite>, BuildError> {
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
pub fn known(value: bool, unknown: bool) -> Option<bool> {
    (value || !unknown).then_some(value)
}

/// Whether `binding` inherits the reads of `previous`, the same-scope binding it shadows: Ruff
/// copies them unless that one is a deletion or an `except` name its handler has unbound. A
/// nested function's `nonlocal` bindings, which Ruff keeps in the nested scope, inherit only from
/// each other.
pub fn inherits(previous: &Binding, binding: &Binding) -> bool {
    previous.deferred == binding.deferred
        && !previous.is_deletion()
        && previous.until.is_none_or(|until| binding.offset < until)
}

/// The steps an index's build spends, against its cap and the evaluation deadline.
pub struct Steps<'d> {
    pub cost: usize,
    pub cap: usize,
    pub deadline: &'d dyn Fn() -> bool,
}

impl Steps<'_> {
    pub fn take(&mut self) -> Result<(), BuildError> {
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
    pub fn crosses_function(&self, inner: u32, outer: u32) -> bool {
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
    pub fn shadowed_by_local(&self, reference: &super::Reference, name: &str) -> Option<Shadowed> {
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
    pub fn reads(&self, reference: &super::Reference, name: &str) -> Vec<(u32, usize)> {
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
    pub fn submodule_alias<'t>(
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

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)

// TypeScript's and JavaScript's usage index (moved from `usage.rs` by the crate split). (`lang_ecma`, its crate's).
// cratify: `lang_ecma` is rerail-lang-ecma's (it mounts it)

// Rust's usage index (moved from `usage.rs` by the crate split). (`lang_rust`, its crate's).
// cratify: `lang_rust` is rerail-lang-rust's (it mounts it)

// PHP's usage index (moved from `usage.rs` by the crate split). (`lang_php`, its crate's).
// cratify: `lang_php` is rerail-lang-php's (it mounts it)

// SQL's usage index (moved from `usage.rs` by the crate split). (`lang_sql`, its crate's).
// cratify: `lang_sql` is rerail-lang-sql's (it mounts it)

// Python's usage index (moved from `usage.rs` by the crate split). (`lang_python`, its crate's).
// cratify: `lang_python` is rerail-lang-python's (it mounts it)
