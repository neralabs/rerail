//! File-local name resolution: one lexical scope table per parsed file, built on first use by a
//! `resolves` rule and shared by every clause that evaluates the same file revision.
//!
//! Python follows Ruff's semantic model: module and class bodies see only bindings made earlier in
//! document order, and function and lambda bodies are resolved after the whole module (Ruff defers
//! them), so they see every binding of the scopes that enclose them. TypeScript follows eslint-scope
//! and typescript-eslint: lexical, hoisted, with separate value and type namespaces. Neither is flow
//! sensitive: branches do not end a binding. Python's unconditional `del` does, as Ruff's
//! `Deletion` binding: order-based, not flow.

pub(crate) mod ecma;
mod lookup;
pub(crate) mod options;
pub(crate) mod php;
pub(crate) mod php_names;
pub(crate) mod python;
pub(crate) mod usage;

pub(crate) use lookup::Lookup;

use crate::code_rule_dsl::identifier::{name_in, nfkc};
use crate::code_rule_dsl::language::adapter::{Adapter, ScopeRules, with_adapter};
use crate::code_rule_dsl::language::{Embedded, Family, GrammarDialect};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::value::{Followed, NodeRef};
use ast_grep_core::{Doc, Node};
use ast_grep_language::LanguageExt;
use foldhash::{HashMap, HashMapExt};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

/// The value namespace: variables, functions, parameters, imports, classes, enums.
pub(crate) const VALUE: u8 = 1;
/// TypeScript's type namespace: interfaces, type aliases, type parameters, classes, enums, imports.
pub(crate) const TYPE: u8 = 2;

/// Nesting deeper than this fails the build instead of risking the stack.
const MAX_BUILD_DEPTH: u32 = 2_000;
/// Stack the recursive build may use below its entry point.
const MAX_BUILD_STACK_BYTES: usize = 512 << 10;
/// Nodes between two reads of the evaluation deadline.
const DEADLINE_INTERVAL: usize = 1_024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ScopeKind {
    Module,
    Class,
    Function,
    Lambda,
    Comprehension,
    /// Python's annotation scope for PEP 695 type parameters; TypeScript's scope of a type-level
    /// declaration or signature.
    TypeParams,
    Block,
    Namespace,
    /// A TypeScript `declare global` body: what it declares augments the global scope, so it
    /// shadows nothing in this file.
    GlobalAugmentation,
    /// The scope that holds a named TypeScript function expression's own name, around its function
    /// scope (eslint-scope's `function-expression-name`).
    FunctionName,
}

/// How a name is bound: the construct its site is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Form {
    /// Python `x = v` (each target of `a = b = v` too); TypeScript `var`, `let` and `const`
    /// declarators.
    Assignment,
    /// Python `x: T = v`.
    Annotated,
    /// Python's bare `x: T` (Ruff's `Annotation`): not assigned, so a load passes over it; made
    /// only where the scope binds the name nowhere earlier.
    Annotation,
    /// Python `x += v`.
    Augmented,
    /// Python `(x := v)`.
    Named,
    /// A `for` target: Python loops and comprehension clauses, TypeScript `for ... in/of`.
    For,
    /// Python `with ... as x`.
    With,
    /// Python `except ... as e`; TypeScript `catch (e)`.
    Except,
    Import,
    /// A function, class, enum, namespace, type alias or interface declaration.
    Definition,
    Parameter,
    /// A Python `global` statement in a function or class.
    Global,
    /// A Python `match` capture.
    Match,
    /// A type parameter, `infer` name, mapped-type key or enum member.
    TypeParameter,
    /// Python `type X = ...`.
    TypeAlias,
    /// A name no construct of the file binds: Ruff's `global` placeholder, TypeScript's
    /// `arguments`.
    Implicit,
    /// Python's unconditional `del x` (Ruff's `Deletion`): it unbinds the name, so a later read
    /// in the scope resolves to nothing.
    Deletion,
    /// A PHP `static $x` declaration.
    Static,
    /// A private field (`#a = 1`), which no scope table holds (`usage::PrivateMember`).
    Field,
    /// A private method (`#a() {}`).
    Method,
    /// A private getter or setter (`get #a()`, `set #a(v)`).
    Accessor,
}

/// Where and how a binding binds its name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Site {
    pub(crate) form: Form,
    /// The construct that binds the name: the assignment, declarator, parameter, import item,
    /// `as` pattern, definition, loop or `global` statement.
    pub(crate) node: Option<NodeRef>,
    /// The value assigned to the name, for the forms that carry one: a plain Python name
    /// assignment, annotated assignment or walrus, or an unpacked name's element of a tuple or
    /// list display; a TypeScript declarator of a plain name with
    /// an initializer (`ScopeTable::value` adds TypeScript's write rule).
    pub(crate) value: Option<NodeRef>,
    /// The name's annotation, without its `:`: a Python annotated assignment or typed parameter;
    /// a TypeScript declarator, parameter or catch parameter.
    pub(crate) annotation: Option<NodeRef>,
}

impl Site {
    pub(crate) const IMPLICIT: Site = Site {
        form: Form::Implicit,
        node: None,
        value: None,
        annotation: None,
    };
}

#[derive(Clone, Debug)]
pub(crate) struct Binding {
    /// Where the binding takes effect (Python ordering); TypeScript ignores it.
    pub(crate) offset: usize,
    pub(crate) spaces: u8,
    /// Made from a function body into an enclosing scope (`global`/`nonlocal`): Ruff visits it after
    /// the enclosing body, so only other deferred bodies see it.
    pub(crate) deferred: bool,
    pub(crate) import: Option<Import>,
    pub(crate) origin: Origin,
    /// Where Python unbinds the name: the end of its `except` handler. A later read, and any read
    /// from a deferred body, sees what the name meant before.
    pub(crate) until: Option<usize>,
    pub(crate) site: Site,
    /// TypeScript: references that write the name (assignments, updates, `for ... in/of`
    /// targets), its declarator's initializer aside.
    pub(crate) writes: u32,
    /// TypeScript: a reference mutates a property of the value (`x.a = 1`, `x[0]++`,
    /// `x.push(1)`), as eslint-utils' `hasMutationInProperty` reads it.
    pub(crate) mutated: bool,
    /// The name node, where a linter reports the binding: the identifier; for an unaliased
    /// Python import item its `dotted_name` (`os.path`, `x`), for an aliased one the alias.
    pub(crate) name: Option<NodeRef>,
    /// Python: the innermost branch of an `if`/`elif`/`else`, `try` clause, `with` body or `match`
    /// case the binding's statement sits in (Ruff's branch path); 0 outside any. A `try` body and
    /// its `else` share one.
    pub(crate) branch: u32,
    /// Python: made under a `global` or `nonlocal` declaration (Ruff's `GLOBAL`/`NONLOCAL`
    /// flags): the `global` statement itself, a later binding of a name its scope declares
    /// `global`, or a `nonlocal` name's binding.
    pub(crate) outer: bool,
    /// Python: an explicit re-export, `import a as a` or `from m import a as a`.
    pub(crate) explicit_export: bool,
    /// Python: `import a.b` without an alias binds `a`; this is the submodule it imports.
    pub(crate) submodule: Option<Box<str>>,
    /// Python: which import statement form binds the name (Ruff's binding kinds).
    pub(crate) import_kind: Option<ImportKind>,
    /// Python: a nested function's `nonlocal` statement names this binding (Ruff's rebinding
    /// scopes), so a bare annotation may be initialized there.
    pub(crate) rebound: bool,
}

/// The import forms Ruff binds apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ImportKind {
    /// `import m`, `import m as x`, `import a.b as x` (Ruff's `Import`).
    Module,
    /// `import a.b` without an alias, which binds `a` (Ruff's `SubmoduleImport`).
    Submodule,
    /// `from m import x` (Ruff's `FromImport`).
    Member,
    /// `from __future__ import x` (Ruff's `FutureImport`).
    Future,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Origin {
    Bound,
    /// A Python `global` statement (Ruff's `BindingKind::Global`): it binds the name in its own
    /// scope, and reads through it continue in the module.
    Global,
    /// Ruff's module placeholder for a name a function or class declares `global` while the module
    /// binds nothing (`SemanticModel::set_globals`): it keeps F821 silent and shadows nothing.
    Declared,
}

impl Binding {
    /// Its value node `value`, followed (`value.follow`).
    pub(crate) fn followed(&self, value: NodeRef) -> Followed {
        Followed {
            value,
            mutated: self.mutated,
            // A declarator's or an assignment's own value ends where it does.
            destructured: self.site.node.is_some_and(|site| site.end != value.end),
        }
    }

    /// A binding with no import, origin or Python flags.
    pub(crate) fn new(offset: usize, spaces: u8, site: Site) -> Self {
        Self {
            offset,
            spaces,
            deferred: false,
            import: None,
            origin: Origin::Bound,
            until: None,
            site,
            writes: 0,
            mutated: false,
            name: None,
            branch: 0,
            outer: false,
            explicit_export: false,
            submodule: None,
            import_kind: None,
            rebound: false,
        }
    }

    /// Whether the binding hides a global or builtin of the same name.
    pub(crate) fn shadows(&self) -> bool {
        self.origin != Origin::Declared
    }

    /// Whether the name is unbound here: a Python `del` removed it.
    pub(crate) fn is_deletion(&self) -> bool {
        self.site.form == Form::Deletion
    }

    /// Whether this is a bare Python annotation, `x: T`, which binds no value.
    pub(crate) fn is_annotation(&self) -> bool {
        self.site.form == Form::Annotation
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Import {
    /// The qualified name the bound name denotes (`import a.b` binds `a` to `a`).
    Qualified(Box<str>),
    /// A relative import, `level` dots and then `rest` (`from ..a import b`: 2, `a.b`). It
    /// resolves only against a file's module path (Ruff's `from_relative_import`).
    Relative { level: usize, rest: Box<str> },
    /// A Python `def` or `class`: Ruff qualifies it by the file's module path, or by the file's
    /// stem outside a package.
    Definition,
}

pub(crate) struct Scope {
    pub(crate) kind: ScopeKind,
    #[allow(dead_code)]
    pub(crate) range: Range<usize>,
    pub(crate) parent: Option<u32>,
    pub(crate) bindings: HashMap<Box<str>, Vec<Binding>>,
    /// Python `nonlocal` declarations of this scope.
    pub(crate) nonlocals: BTreeSet<Box<str>>,
    /// Python: where each `from m import *` of this scope takes effect, in order.
    pub(crate) stars: Vec<usize>,
}

/// A Python import statement's top-level module and where Ruff visits it: the module body first
/// (`depth` 0), then function bodies one nesting level at a time, each level in document order.
#[derive(Clone, Debug)]
pub(crate) struct ModuleImport {
    pub(crate) module: Box<str>,
    pub(crate) depth: u32,
    pub(crate) offset: usize,
}

impl ScopeTable {
    /// Ruff's `seen_module`: whether an import of one of `modules` (top-level names) is visited
    /// before a node Ruff visits at `depth` (function bodies around it; `u32::MAX` for a lambda
    /// body or a type parameter's bound, which Ruff visits after every function) and `offset`.
    pub(crate) fn seen_module(
        &self,
        modules: &BTreeSet<String>,
        depth: u32,
        offset: usize,
    ) -> bool {
        self.module_imports.iter().any(|import| {
            modules.contains(import.module.as_ref())
                && (import.depth, import.offset) < (depth, offset)
        })
    }
}

/// How a reference reads its name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Access {
    /// A load, TypeScript's writes included.
    Load,
    /// Python `del x`: Ruff resolves it in the current scope only, and counts it as a use.
    Del,
    /// A Python `nonlocal x` statement's name: it reads the enclosing binding (Ruff marks it
    /// used). Not a reference position for `resolves`.
    Nonlocal,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Reference {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) scope: u32,
    /// The namespaces the reference reads: `VALUE`, `TYPE`, or both (an export specifier).
    pub(crate) spaces: u8,
    /// Where the reference is read, for Python ordering: a comprehension's element is read after
    /// its clauses, so it sees what a walrus in a condition binds.
    pub(crate) at: usize,
    /// Read after the whole module, like a function body: an annotation under
    /// `from __future__ import annotations`.
    pub(crate) deferred: bool,
    /// Python: a forward reference (a typing-only annotation read after the module, a name in a
    /// string type definition), which Ruff resolves in the module scope first (PEP 563, Ruff's
    /// `in_forward_reference`).
    pub(crate) forward: bool,
    /// Python: read in the type scope Ruff opens around a class or function definition (a class's
    /// bases and keywords, a function's parameter and return annotations), so no reference of the
    /// scope around the definition for Ruff's `undefined_local`.
    pub(crate) type_scope: bool,
    /// TypeScript: the reference writes the name (`x = 1`, `x++`, `[x] = a`, `for (x of a)`).
    pub(crate) write: bool,
    /// TypeScript: the reference heads a member chain that is written, updated or called with a
    /// mutating array method (`x.a = 1`, `x[0]++`, `x.push(1)`).
    pub(crate) mutates: bool,
    pub(crate) access: Access,
}

/// Names in string type definitions have ranges of their own: this bit, then the annotation's
/// index (`ScopeTable::annotations`) above the low 24 bits, which hold the offset into its text.
/// No source position has them.
pub(crate) const ANNOTATION_RANGE: usize = 1 << 62;
const ANNOTATION_SHIFT: u32 = 24;

/// Where the names of the annotation with this index start.
pub(crate) fn annotation_offset(index: usize) -> usize {
    ANNOTATION_RANGE | (index << ANNOTATION_SHIFT)
}

/// A string type definition the table parsed and read (Ruff's `string_type_definitions`).
pub(crate) struct Annotation {
    /// The string literal's range: in the source, or, nested in another annotation, in that one's
    /// range space.
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// The parsed expression, or one `ERROR` node where Ruff reports a syntax error (F722).
    pub(crate) tree: Arc<crate::code_rule_dsl::literal::SubTree>,
    /// The scope the string is read in.
    pub(crate) scope: u32,
}

/// ESLint `/* global */` comments of a file, for predicates that opt in to them.
#[derive(Default, Debug)]
pub(crate) struct Directives {
    pub(crate) declared: BTreeSet<String>,
    pub(crate) off: BTreeSet<String>,
}

pub(crate) struct ScopeTable {
    pub(crate) language: GrammarDialect,
    /// The family of `language`, whose `ScopeRules` read the table.
    pub(crate) family: Family,
    pub(crate) scopes: Vec<Scope>,
    /// Sorted by start byte; at most one reference starts at a byte.
    pub(crate) references: Vec<Reference>,
    pub(crate) directives: Directives,
    /// Nodes the build visited: what a clause pays for the table (`maxVisitedNodes`).
    pub(crate) cost: usize,
    /// How the table read its file.
    pub(crate) reading: Reading,
    /// Python: the string type definitions, parsed; their names are references with ranges of
    /// their own (`annotation_offset`). Sorted by start.
    pub(crate) annotations: Vec<Annotation>,
    /// Python: module-level statements that assign `__all__` (`=`, `+=`, `: T =`).
    pub(crate) dunder_all: Vec<NodeRef>,
    /// TypeScript: the scope of each JSX opening or self-closing element, in document order, and
    /// whether it is a fragment (`<>`), where typescript-eslint references the JSX pragma.
    pub(crate) jsx: Vec<(u32, bool)>,
    /// TypeScript: the scopes a direct `eval(...)` or a `with` statement runs in, where a read may
    /// be unseen.
    pub(crate) dynamic: Vec<u32>,
    /// Python: the scopes that call `locals()` directly (Ruff's `USES_LOCALS`).
    pub(crate) locals_scopes: Vec<u32>,
    /// Python: each absolute import's top-level module, where Ruff's traversal visits it (Ruff's
    /// `add_module`, which `seen_module` reads).
    pub(crate) module_imports: Vec<ModuleImport>,
    /// Python usedness, built on first use.
    pub(crate) usage: OnceLock<Arc<usage::Usage>>,
    /// PHP: what reaches each read of a variable.
    pub(crate) php: Option<Box<php::Flow>>,
}

/// How a table reads its file, beyond the grammar: tables of one file under different readings
/// are kept apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Reading {
    /// Python: every annotation is read after the module (a target of 3.14 or later).
    pub(crate) deferred_annotations: bool,
    /// TypeScript: blocks and switch bodies are scopes, as eslint-scope nests them from
    /// ecmaVersion 6 (ES2015); before it only functions and catch clauses are.
    pub(crate) block_scopes: bool,
    /// Python: the file is a `.pyi` stub (the host's path says so), which Ruff reads with stub
    /// semantics: type definitions and class bases are read after the module, annotations are
    /// forward references, a bare annotation binds, and a read before a `del` sees the binding the
    /// `del` removes.
    pub(crate) stub: bool,
    /// Python: the target version's minor, which a stub's typing-aware reading needs (the builtins
    /// a generic subscript may name, TypedDict's keyword fields before 3.13). 0 outside stubs,
    /// so their tables are shared across versions.
    pub(crate) python_minor: u8,
    /// Python: Ruff's typing settings, which change what is `typing`'s and so which strings are
    /// type definitions and what a stub reads after the module.
    pub(crate) typing: Option<&'static options::PythonTyping>,
}

impl Default for Reading {
    fn default() -> Self {
        Self {
            deferred_annotations: false,
            block_scopes: true,
            stub: false,
            python_minor: 0,
            typing: None,
        }
    }
}

/// Why a build stopped. Any of them fails the clause for the file: a partial table could turn
/// "shadowed" into "global", which a `not` would report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BuildError {
    Budget,
    Depth,
    Deadline,
}

impl ScopeTable {
    /// Builds the table of `root`'s file, visiting at most `cap` nodes.
    #[cfg(test)]
    pub(crate) fn build<D: Doc>(
        root: Node<'_, D>,
        language: GrammarDialect,
        cap: usize,
        deadline: &dyn Fn() -> bool,
    ) -> Result<Self, BuildError> {
        Self::build_with(root, language, cap, deadline, Reading::default())
    }

    /// The binding a `usage::BindingId` names.
    pub(crate) fn binding_of(&self, id: usage::BindingId<'_>) -> &Binding {
        &self.scopes[id.scope as usize].bindings[id.name][id.index]
    }

    /// The bindings of `id`'s name in its scope, in Ruff's order.
    pub(crate) fn chain(&self, id: usage::BindingId<'_>) -> &[Binding] {
        &self.scopes[id.scope as usize].bindings[id.name]
    }

    /// Builds the table of `root`'s file under `reading`, visiting at most `cap` nodes.
    #[cfg(test)]
    pub(crate) fn build_with<D: Doc>(
        root: Node<'_, D>,
        language: GrammarDialect,
        cap: usize,
        deadline: &dyn Fn() -> bool,
        reading: Reading,
    ) -> Result<Self, BuildError> {
        Self::build_in(root, language, cap, deadline, reading, None)
    }

    /// `build_with` for a Python file at `module`, against which its relative imports and
    /// definitions are qualified where Ruff's typing settings name them.
    pub(crate) fn build_in<D: Doc>(
        root: Node<'_, D>,
        language: GrammarDialect,
        cap: usize,
        deadline: &dyn Fn() -> bool,
        reading: Reading,
        module: Option<&ModulePath>,
    ) -> Result<Self, BuildError> {
        Self::build_with_names(root, language, cap, deadline, reading, module, None)
    }

    /// `build_in`, where a PHP or Blade file's flow reads `php_names`, the file's names built
    /// apart, rather than its own. A table's `cost` never holds its names' (`PhpNames::cost`):
    /// they are charged where they are built, once per file, whoever reads them.
    pub(crate) fn build_with_names<D: Doc>(
        root: Node<'_, D>,
        language: GrammarDialect,
        cap: usize,
        deadline: &dyn Fn() -> bool,
        reading: Reading,
        module: Option<&ModulePath>,
        php_names: Option<Arc<php_names::PhpNames>>,
    ) -> Result<Self, BuildError> {
        let family = Family::of(language);
        let mut builder = Builder {
            module: module.cloned(),
            table: ScopeTable {
                language,
                family,
                scopes: Vec::new(),
                references: Vec::new(),
                directives: Directives::default(),
                cost: 0,
                reading,
                annotations: Vec::new(),
                dunder_all: Vec::new(),
                jsx: Vec::new(),
                dynamic: Vec::new(),
                locals_scopes: Vec::new(),
                module_imports: Vec::new(),
                usage: OnceLock::new(),
                php: None,
            },
            current: 0,
            position: None,
            deferring: false,
            forwarding: false,
            typing: false,
            type_roots: Vec::new(),
            strings: HashMap::new(),
            branch: 0,
            branches: 0,
            cap,
            depth: 0,
            stack_base: stack_address(),
            deadline,
            php_names,
        };
        builder.push(ScopeKind::Module, root.range());
        with_adapter!(family, A => <<A as Adapter>::Scope as ScopeRules>::visit(&mut builder, &root, reading))?;
        let mut table = builder.table;
        table.sort_references();
        with_adapter!(family, A => <<A as Adapter>::Scope as ScopeRules>::finish(&mut table, &root));
        Ok(table)
    }

    /// Marks each Python binding a nested `nonlocal` statement names (`Binding::rebound`).
    fn mark_rebound(&mut self, text: &str, base: usize) {
        let mut rebound = Vec::new();
        for reference in &self.references {
            if reference.access != Access::Nonlocal {
                continue;
            }
            let Some(name) = text.get(reference.start - base..reference.end - base) else {
                continue;
            };
            let name = nfkc(name);
            if let Some(((scope, index), _)) = self.walk(reference, &name, true).found {
                rebound.push((scope, name.into_owned(), index));
            }
        }
        for (scope, name, index) in rebound {
            if let Some(binding) = self.scopes[scope as usize]
                .bindings
                .get_mut(name.as_str())
                .and_then(|bindings| bindings.get_mut(index))
            {
                binding.rebound = true;
            }
        }
    }

    /// Sorts the references by start. A node visited twice keeps its first reference, and is a write
    /// if either visit wrote.
    fn sort_references(&mut self) {
        self.references.sort_by_key(|reference| reference.start);
        let mut references: Vec<Reference> = Vec::with_capacity(self.references.len());
        for reference in std::mem::take(&mut self.references) {
            match references.last_mut() {
                Some(last) if last.start == reference.start => {
                    last.write |= reference.write;
                    last.mutates |= reference.mutates;
                }
                _ => references.push(reference),
            }
        }
        self.references = references;
    }

    /// The string type definition whose literal spans exactly `range`.
    pub(crate) fn annotation_at(&self, range: &Range<usize>) -> Option<&Annotation> {
        let index = self
            .annotations
            .binary_search_by_key(&range.start, |annotation| annotation.start)
            .ok()?;
        let annotation = &self.annotations[index];
        (annotation.end == range.end).then_some(annotation)
    }

    /// Where a reference is written in the source: its start, or for a name in a string type
    /// definition the start of the outermost string holding it.
    fn source_start(&self, reference: &Reference) -> usize {
        let mut start = reference.start;
        for _ in 0..self.annotations.len() {
            if start & ANNOTATION_RANGE == 0 {
                return start;
            }
            let offset = annotation_offset((start & !ANNOTATION_RANGE) >> ANNOTATION_SHIFT);
            let Some(annotation) = self
                .annotations
                .iter()
                .find(|annotation| annotation.tree.offset() == offset)
            else {
                break;
            };
            start = annotation.start;
        }
        start
    }

    /// A reference's name: the source text it spans (`text` is the file's from `base`), or the
    /// text of the annotation it lies in; a Python name NFKC-normalized, as the table's keys are.
    pub(crate) fn reference_name<'a>(
        &'a self,
        reference: &Reference,
        text: &'a str,
        base: usize,
    ) -> Option<Cow<'a, str>> {
        let raw = if reference.start & ANNOTATION_RANGE == 0 {
            text.get(reference.start - base..reference.end - base)?
        } else {
            let index = (reference.start & !ANNOTATION_RANGE) >> ANNOTATION_SHIFT;
            let offset = annotation_offset(index);
            let annotation = self
                .annotations
                .iter()
                .find(|annotation| annotation.tree.offset() == offset)?;
            annotation
                .tree
                .text()
                .get(reference.start - offset..reference.end - offset)?
        };
        Some(name_in(self.language, Cow::Borrowed(raw)))
    }

    /// Charges each TypeScript write and property mutation to the binding it resolves to.
    fn count_writes(&mut self, text: &str, base: usize) {
        let mut charges = Vec::new();
        for reference in &self.references {
            if !(reference.write || reference.mutates) {
                continue;
            }
            let Some(name) = text.get(reference.start - base..reference.end - base) else {
                continue;
            };
            if let Some((scope, index)) = self.resolve_lexical_index(reference, name) {
                charges.push((
                    scope,
                    name.to_owned(),
                    index,
                    reference.write,
                    reference.mutates,
                ));
            }
        }
        for (scope, name, index, write, mutates) in charges {
            if let Some(binding) = self.scopes[scope as usize]
                .bindings
                .get_mut(name.as_str())
                .and_then(|bindings| bindings.get_mut(index))
            {
                binding.writes += u32::from(write);
                binding.mutated |= mutates;
            }
        }
    }

    /// The reference whose identifier spans exactly `range`, if that node is in a reference position.
    pub(crate) fn reference(&self, range: &Range<usize>) -> Option<&Reference> {
        let index = self
            .references
            .binary_search_by_key(&range.start, |reference| reference.start)
            .ok()?;
        let reference = &self.references[index];
        (reference.end == range.end && reference.access != Access::Nonlocal).then_some(reference)
    }

    /// The binding `name` resolves to at `reference`, or `None` for a global or builtin. A Python
    /// `del` resolves in its own scope only; a `Deletion` binding may be the answer.
    pub(crate) fn resolve(&self, reference: &Reference, name: &str) -> Option<&Binding> {
        with_adapter!(self.family, A => <<A as Adapter>::Scope as ScopeRules>::resolve(self, reference, name))
    }

    /// PHP: the one binding every path reaches a read with, as its scope and its index among that
    /// scope's bindings of the name. `None` where no binding reaches, where several may, where
    /// some path reaches none, or where PHP's dynamic symbol table may decide.
    pub(crate) fn php_index(&self, reference: &Reference) -> Option<(u32, usize)> {
        let read = self.php.as_deref()?.read(reference.start)?;
        match (read.bindings.as_ref(), read.defined) {
            ([binding], php::Defined::Yes) => Some(*binding),
            _ => None,
        }
    }

    /// Whether `name` at `reference` has no binding: none resolves, or a `del` unbound it
    /// (Ruff's F821, builtins and star imports aside).
    #[cfg(test)]
    pub(crate) fn unbound(&self, reference: &Reference, name: &str) -> bool {
        if reference.access != Access::Del
            && let Some(annotation) = with_adapter!(self.family, A => <<A as Adapter>::Scope as ScopeRules>::unbound_annotation(self, reference, name))
        {
            return !self.binding_at(annotation, name).rebound;
        }
        self.unbound_locally(reference, name, false)
            || self
                .resolve(reference, name)
                .is_none_or(|binding| binding.is_deletion())
    }

    /// Whether Python's lookup of `name` at `reference` stops at something that unbinds it,
    /// builtins included: a deletion, an `except` name its handler unbound (when `builtin` says
    /// the name is no builtin, which the handler's lookup would have found), or a function's bare
    /// annotation no nested `nonlocal` may initialize.
    #[cfg(test)]
    pub(crate) fn unbound_locally(&self, reference: &Reference, name: &str, builtin: bool) -> bool {
        with_adapter!(self.family, A => <<A as Adapter>::Scope as ScopeRules>::unbound_locally(self, reference, name, builtin))
    }

    /// Ruff's lookup of `name` at `reference` (`unbound_locally`).
    #[cfg(test)]
    pub(crate) fn ruff_unbound_locally(
        &self,
        reference: &Reference,
        name: &str,
        builtin: bool,
    ) -> bool {
        if reference.access != Access::Del {
            let walk = self.walk(reference, name, true);
            if let Some(annotation) = walk.unbound {
                return !self.binding_at(annotation, name).rebound;
            }
            if walk.found.is_none() && self.unbound_exception(reference, name) {
                return !builtin;
            }
        }
        self.resolve(reference, name)
            .is_some_and(|binding| binding.is_deletion())
    }

    /// Whether a star import may bind `name` where `reference` reads it: one of an enclosing
    /// scope the read sees (not a class body it cannot see), made earlier or, from a function
    /// body, anywhere (Ruff's `uses_star_imports` along `resolve_load`).
    pub(crate) fn star_visible(&self, reference: &Reference) -> bool {
        let mut crossed = reference.deferred;
        let mut current = Some(reference.scope);
        let mut previous: Option<ScopeKind> = None;
        let mut first = true;
        while let Some(index) = current {
            let scope = &self.scopes[index as usize];
            let hidden =
                scope.kind == ScopeKind::Class && !first && previous != Some(ScopeKind::TypeParams);
            if !hidden
                && scope
                    .stars
                    .iter()
                    .any(|offset| crossed || *offset <= reference.at)
            {
                return true;
            }
            crossed |= matches!(scope.kind, ScopeKind::Function | ScopeKind::Lambda);
            previous = Some(scope.kind);
            first = false;
            current = scope.parent;
        }
        false
    }

    /// Whether Python binds `name` implicitly where `reference` reads it, so Ruff never reports it
    /// undefined: `__module__` and `__qualname__` in a class body, `__class__` in a method or a
    /// lambda inside a class (Ruff's `DunderClassCell`), `__path__` in a package's `__init__`.
    pub(crate) fn implicit(&self, reference: &Reference, name: &str, init: bool) -> bool {
        match name {
            "__module__" | "__qualname__" => {
                self.scopes[reference.scope as usize].kind == ScopeKind::Class
            }
            "__class__" => {
                let mut current = Some(reference.scope);
                while let Some(index) = current {
                    let scope = &self.scopes[index as usize];
                    let parent = scope.parent.map(|parent| &self.scopes[parent as usize]);
                    let cell = match scope.kind {
                        ScopeKind::Function => {
                            parent.is_some_and(|parent| parent.kind == ScopeKind::Class)
                        }
                        ScopeKind::Lambda => self.encloses_class(scope.parent),
                        _ => false,
                    };
                    if cell {
                        return true;
                    }
                    current = scope.parent;
                }
                false
            }
            "__path__" => init,
            _ => false,
        }
    }

    fn encloses_class(&self, mut current: Option<u32>) -> bool {
        while let Some(index) = current {
            let scope = &self.scopes[index as usize];
            if scope.kind == ScopeKind::Class {
                return true;
            }
            current = scope.parent;
        }
        false
    }

    /// The binding `name` at a Python `reference` resolves to, as its scope and its index among
    /// that scope's bindings of the name (in Ruff's order; see `build_with`).
    pub(crate) fn resolve_python_index(
        &self,
        reference: &Reference,
        name: &str,
    ) -> Option<(u32, usize)> {
        if reference.access == Access::Del {
            let scope = &self.scopes[reference.scope as usize];
            if !scope.nonlocals.contains(name) {
                return Self::visible(scope, name, reference.deferred, reference.at)
                    .map(|index| (reference.scope, index));
            }
        }
        self.resolve_load_index(reference, name)
    }

    pub(crate) fn binding_at(&self, (scope, index): (u32, usize), name: &str) -> &Binding {
        &self.scopes[scope as usize].bindings[name][index]
    }

    /// Ruff's `resolve_del`: the binding of the reference's own scope so far. A `nonlocal` name
    /// reads the enclosing function's.
    pub(crate) fn resolve_del(&self, reference: &Reference, name: &str) -> Option<&Binding> {
        self.resolve_python_index(reference, name)
            .map(|found| self.binding_at(found, name))
    }

    /// Ruff's `resolve_load`, with function bodies deferred to the end of the module: a class scope
    /// is visible only to its own body (and to the type-parameter scope directly inside it), a
    /// `global` binding continues in the module, `nonlocal` skips its own scope. Before the lookup
    /// crosses a function boundary it sees the bindings made so far; after, every binding.
    pub(crate) fn resolve_python(&self, reference: &Reference, name: &str) -> Option<&Binding> {
        self.resolve_load_index(reference, name)
            .map(|found| self.binding_at(found, name))
    }

    /// `resolve_load_index`, where `stop_at_global` keeps a `global` binding instead of continuing
    /// in the module (Ruff's `lookup_symbol`).
    fn resolve_python_scope(
        &self,
        reference: &Reference,
        name: &str,
        stop_at_global: bool,
    ) -> Option<(u32, usize)> {
        let (found, module) = self.resolve_load_pair(reference, name)?;
        Some(if stop_at_global {
            found
        } else {
            module.unwrap_or(found)
        })
    }

    fn resolve_load_index(&self, reference: &Reference, name: &str) -> Option<(u32, usize)> {
        self.resolve_load_pair(reference, name)
            .map(|(found, module)| module.unwrap_or(found))
    }

    /// The binding the scope walk finds and, for a `global` declaration, the module binding the
    /// read continues to (Ruff's `lookup_symbol`: a bare annotation is passed over).
    #[allow(clippy::type_complexity)]
    fn resolve_load_pair(
        &self,
        reference: &Reference,
        name: &str,
    ) -> Option<((u32, usize), Option<(u32, usize)>)> {
        self.walk(reference, name, false).found
    }

    /// The scope walk of a Python read. A load passes over a bare annotation of a module or class
    /// (Ruff's `resolve_load` still counts it read); with `load`, one of a function stops it (Ruff
    /// reads such a name as an unbound local), otherwise it is passed over too (`lookup_symbol`).
    /// A `nonlocal` statement finds a bare annotation.
    pub(crate) fn walk(&self, reference: &Reference, name: &str, load: bool) -> Walk {
        let mut walk = Walk::default();
        if let Some(found) = self.forward_module(reference, name) {
            walk.found = Some(((0, found), None));
            return walk;
        }
        let mut crossed = reference.deferred;
        let mut current = Some(reference.scope);
        let mut previous: Option<ScopeKind> = None;
        let mut first = true;
        while let Some(index) = current {
            let scope = &self.scopes[index as usize];
            let hidden =
                scope.kind == ScopeKind::Class && !first && previous != Some(ScopeKind::TypeParams);
            let function = matches!(scope.kind, ScopeKind::Function | ScopeKind::Lambda);
            // Ruff binds a nested `nonlocal` name in the nested scope, so the statement reads the
            // enclosing function's own bindings only.
            let found = if reference.access == Access::Nonlocal {
                Self::own(scope, name)
            } else {
                Self::visible(scope, name, crossed, reference.at)
            };
            if !hidden
                && !scope.nonlocals.contains(name)
                && let Some(found) = found
            {
                let mut found = found;
                let mut binding = &scope.bindings[name][found];
                // A stub hides a name from re-export with `del` yet reads it before: a read
                // written before the `del` resolves to the binding it removed (Ruff's stub rule).
                if self.reading.stub
                    && binding.is_deletion()
                    && found > 0
                    && binding
                        .name
                        .is_some_and(|node| self.source_start(reference) < node.start as usize)
                {
                    let previous = &scope.bindings[name][found - 1];
                    if !previous.is_annotation()
                        && !previous.is_deletion()
                        && previous.until.is_none()
                    {
                        found -= 1;
                        binding = previous;
                    }
                }
                if binding.is_annotation()
                    && !self.reading.stub
                    && reference.access != Access::Nonlocal
                {
                    walk.annotations.push((index, found));
                    if load && scope.kind == ScopeKind::Function {
                        walk.unbound = Some((index, found));
                        return walk;
                    }
                } else if binding.origin != Origin::Global {
                    walk.found = Some(((index, found), None));
                    return walk;
                } else {
                    // Ruff's `Global(Some(id))` reads resolve to the module's binding, but the
                    // placeholder or no binding leaves the `global` binding, which is no builtin.
                    let module =
                        Self::visible(&self.scopes[0], name, crossed || function, reference.at)
                            .filter(|module| {
                                let binding = &self.scopes[0].bindings[name][*module];
                                binding.shadows() && !binding.is_annotation()
                            })
                            .map(|module| (0, module));
                    walk.found = Some(((index, found), module));
                    return walk;
                }
            }
            crossed |= function;
            previous = Some(scope.kind);
            first = false;
            current = scope.parent;
        }
        walk
    }

    /// PEP 563: a forward reference prefers the module's latest binding of the name, unless that
    /// one unbinds it (a bare annotation, a deletion, an `except` name). Ruff then reads it from
    /// the module scope. The binding's index.
    pub(crate) fn forward_module(&self, reference: &Reference, name: &str) -> Option<usize> {
        if !reference.forward || reference.access != Access::Load {
            return None;
        }
        let bindings = self.scopes[0].bindings.get(name)?;
        let last = bindings.last()?;
        (!last.is_annotation() && !last.is_deletion() && last.until.is_none())
            .then(|| bindings.len() - 1)
    }

    /// Whether a Python load of `name` that resolves to nothing stops at an `except` name its
    /// handler has unbound, with nothing bound before it in that scope: Ruff's
    /// `UnboundException(None)`, an undefined name whatever a star import may bind, unless the
    /// name is a builtin, which the handler's lookup would have found (the caller's to check).
    pub(crate) fn unbound_exception(&self, reference: &Reference, name: &str) -> bool {
        let mut crossed = reference.deferred;
        let mut current = Some(reference.scope);
        let mut previous: Option<ScopeKind> = None;
        let mut first = true;
        while let Some(index) = current {
            let scope = &self.scopes[index as usize];
            let hidden =
                scope.kind == ScopeKind::Class && !first && previous != Some(ScopeKind::TypeParams);
            if !hidden
                && !scope.nonlocals.contains(name)
                && scope.bindings.get(name).is_some_and(|bindings| {
                    bindings.iter().any(|binding| {
                        binding.until.is_some_and(|until| {
                            crossed || (!binding.deferred && until <= reference.at)
                        })
                    })
                })
            {
                return true;
            }
            crossed |= matches!(scope.kind, ScopeKind::Function | ScopeKind::Lambda);
            previous = Some(scope.kind);
            first = false;
            current = scope.parent;
        }
        false
    }

    /// The latest binding of `name` that `scope`'s own body makes (none redirected into it).
    fn own(scope: &Scope, name: &str) -> Option<usize> {
        latest(scope.bindings.get(name)?, |binding| {
            !binding.deferred && binding.until.is_none()
        })
    }

    /// The latest binding of `name` in `scope` a read at `at` sees.
    fn visible(scope: &Scope, name: &str, crossed: bool, at: usize) -> Option<usize> {
        latest(scope.bindings.get(name)?, |binding| {
            if crossed {
                binding.until.is_none()
            } else {
                !binding.deferred
                    && binding.offset <= at
                    && binding.until.is_none_or(|until| at < until)
            }
        })
    }

    pub(crate) fn resolve_lexical(&self, reference: &Reference, name: &str) -> Option<&Binding> {
        self.resolve_lexical_scope(reference, name)
            .map(|(_, binding)| binding)
    }

    /// The binding `name` resolves to at a TypeScript `reference`, with the index of the scope that
    /// holds it (0 is the file's top level).
    pub(crate) fn resolve_lexical_scope(
        &self,
        reference: &Reference,
        name: &str,
    ) -> Option<(u32, &Binding)> {
        let (scope, index) = self.resolve_lexical_index(reference, name)?;
        Some((scope, &self.scopes[scope as usize].bindings[name][index]))
    }

    /// The scope holding the binding `name` resolves to at a TypeScript `reference`, and the
    /// binding's index among that scope's bindings of the name.
    pub(crate) fn resolve_lexical_index(
        &self,
        reference: &Reference,
        name: &str,
    ) -> Option<(u32, usize)> {
        let mut current = Some(reference.scope);
        while let Some(index) = current {
            let scope = &self.scopes[index as usize];
            if let Some(found) = scope.bindings.get(name).and_then(|bindings| {
                bindings
                    .iter()
                    .position(|binding| binding.spaces & reference.spaces != 0)
            }) {
                return Some((index, found));
            }
            current = scope.parent;
        }
        None
    }

    /// The file-local binding `name` resolves to at `reference` and the value it carries: none
    /// for a global or builtin, for Ruff's `global` placeholder, which binds nothing, or for a name
    /// an unconditional `del` unbound.
    ///
    /// Python binds a value where the binding the table picks (Ruff's model: the latest one the
    /// read sees) is a plain name assignment, annotated assignment with a value or walrus, or an
    /// unpacked name paired with a display's element. PHP binds one where one binding reaches the
    /// read on every path, and the variable is no reference.
    /// TypeScript binds one where the name has a single definition, a declarator of the plain name
    /// with an initializer, and no reference writes it (eslint-utils' `isEffectivelyConst`).
    pub(crate) fn binding(
        &self,
        reference: &Reference,
        name: &str,
    ) -> Option<(&Binding, Option<NodeRef>)> {
        with_adapter!(self.family, A => <<A as Adapter>::Scope as ScopeRules>::binding(self, reference, name))
    }
}

/// What a Python read's scope walk meets (`ScopeTable::walk`).
#[derive(Default)]
pub(crate) struct Walk {
    /// The bare annotations it passed over or stopped at, which Ruff counts as read.
    pub(crate) annotations: Vec<(u32, usize)>,
    /// A function's bare annotation that stopped a load: the name is an unbound local.
    pub(crate) unbound: Option<(u32, usize)>,
    /// The binding found and, through a `global` declaration, the module binding read.
    #[allow(clippy::type_complexity)]
    pub(crate) found: Option<((u32, usize), Option<(u32, usize)>)>,
}

/// The index of the binding with the greatest offset among those `keep` passes (the last one on a
/// tie, as `max_by_key`).
fn latest(bindings: &[Binding], keep: impl Fn(&Binding) -> bool) -> Option<usize> {
    bindings
        .iter()
        .enumerate()
        .filter(|(_, binding)| keep(binding))
        .max_by_key(|(_, binding)| binding.offset)
        .map(|(index, _)| index)
}

/// An address in the current stack frame.
#[inline(always)]
fn stack_address() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}

/// Kind tags and field ids of one grammar, looked up by the kind and field codes that both
/// tree-sitter trees and tapes carry: a file's own grammar's ids, or an embedded program's as its
/// host's tapes number them (`KindCatalog`).
pub(crate) struct Grammar<T> {
    tags: Vec<T>,
    ts: tree_sitter::Language,
    /// How the host's tapes number the grammar's kinds and fields, where it is embedded.
    numbering: Option<&'static Embedded>,
}

impl<T: Copy + Default> Grammar<T> {
    /// The grammar of `dialect`, read in tapes of `host`: `dialect` itself, or a dialect it
    /// embeds (`GrammarDialect::embeds`).
    fn new(host: GrammarDialect, dialect: GrammarDialect, tag: impl Fn(&str) -> T) -> Self {
        let ts = dialect.get_ts_language();
        let numbering = (host != dialect).then(|| {
            host.kinds()
                .embedding(dialect)
                .expect("a region is in its tape's dialect or one the dialect embeds")
        });
        let count = numbering.map_or(ts.node_kind_count(), |_| host.kind_count());
        let mut tags = vec![T::default(); count];
        for id in 0..ts.node_kind_count() as u16 {
            if let Some(name) = ts.node_kind_for_id(id)
                && ts.node_kind_is_named(id)
                && let Some(slot) = tags.get_mut(usize::from(
                    numbering.map_or(id, |numbering| numbering.kind_code(id)),
                ))
            {
                *slot = tag(name);
            }
        }
        Self {
            tags,
            ts,
            numbering,
        }
    }

    fn tag<D: Doc>(&self, node: &Node<'_, D>) -> T {
        self.tags
            .get(usize::from(node.kind_id()))
            .copied()
            .unwrap_or_default()
    }

    fn field(&self, name: &str) -> u16 {
        self.ts.field_id_for_name(name).map_or(0, |id| {
            self.numbering
                .map_or(id.get(), |numbering| numbering.field_code(id.get()))
        })
    }
}

pub(crate) struct Builder<'d> {
    table: ScopeTable,
    /// Python: the file's module path, when the host or the file's name gives one.
    module: Option<ModulePath>,
    current: u32,
    /// Overrides where references are read (`Reference::at`).
    position: Option<usize>,
    /// Marks references as read after the module (`Reference::deferred`).
    deferring: bool,
    /// Marks references as forward references (`Reference::forward`).
    forwarding: bool,
    /// Marks references as read in a definition's type scope (`Reference::type_scope`).
    typing: bool,
    /// Python: where a type definition may start, for `python::string_type_definitions`.
    type_roots: Vec<python::TypeRoot>,
    /// Python: each string literal's scope, by its range.
    strings: HashMap<(usize, usize), u32>,
    /// The current Python branch (`Binding::branch`), and the last one opened.
    branch: u32,
    branches: u32,
    cap: usize,
    depth: u32,
    stack_base: usize,
    deadline: &'d dyn Fn() -> bool,
    /// PHP: the file's resolved names, where the caller built them (and charged their cost); the
    /// flow builds them otherwise.
    php_names: Option<Arc<php_names::PhpNames>>,
}

impl Builder<'_> {
    /// Accounts for one visited node and one level of recursion; `leave` undoes the level.
    fn enter(&mut self) -> Result<(), BuildError> {
        self.table.cost += 1;
        if self.table.cost > self.cap {
            return Err(BuildError::Budget);
        }
        if self.table.cost.is_multiple_of(DEADLINE_INTERVAL) && (self.deadline)() {
            return Err(BuildError::Deadline);
        }
        self.depth += 1;
        if self.depth > MAX_BUILD_DEPTH
            || self.stack_base.abs_diff(stack_address()) > MAX_BUILD_STACK_BYTES
        {
            return Err(BuildError::Depth);
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn push(&mut self, kind: ScopeKind, range: Range<usize>) -> u32 {
        let parent = (!self.table.scopes.is_empty()).then_some(self.current);
        let index = self.table.scopes.len() as u32;
        self.table.scopes.push(Scope {
            kind,
            range,
            parent,
            bindings: HashMap::new(),
            nonlocals: BTreeSet::new(),
            stars: Vec::new(),
        });
        self.current = index;
        index
    }

    fn pop(&mut self) {
        self.current = self.table.scopes[self.current as usize].parent.unwrap_or(0);
    }

    fn scope(&self, index: u32) -> &Scope {
        &self.table.scopes[index as usize]
    }

    fn bind(&mut self, scope: u32, name: &str, binding: Binding) {
        let bindings = &mut self.table.scopes[scope as usize].bindings;
        // Most names are bound once: a chain starts with room for one binding, and a name already
        // bound allocates no key.
        match bindings.get_mut(name) {
            Some(chain) => chain.push(binding),
            None => {
                bindings.insert(name.into(), vec![binding]);
            }
        }
    }

    fn reference(&mut self, range: Range<usize>, spaces: u8) {
        self.push_reference(range, spaces, false, false);
    }

    /// A reference that writes the name, or mutates a property of its value (TypeScript).
    fn push_reference(&mut self, range: Range<usize>, spaces: u8, write: bool, mutates: bool) {
        self.access_reference(range, spaces, write, mutates, Access::Load);
    }

    fn access_reference(
        &mut self,
        range: Range<usize>,
        spaces: u8,
        write: bool,
        mutates: bool,
        access: Access,
    ) {
        self.table.references.push(Reference {
            start: range.start,
            end: range.end,
            scope: self.current,
            spaces,
            at: self.position.unwrap_or(range.start),
            deferred: self.deferring,
            forward: self.forwarding,
            type_scope: self.typing,
            write,
            mutates,
            access,
        });
    }
}

#[cfg(test)]
mod tests;
