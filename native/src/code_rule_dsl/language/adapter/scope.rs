//! Scope tables, bindings and usage (`resolves`, `binds`): `Semantic::Scope`.

use super::{Adapter, Refusal};

use crate::code_rule_dsl::matcher::{ScopeName, Unresolved, file_module, file_source_type_for};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::{
    DEFAULT_PYTHON_MINOR, EsGlobals, LanguageOptions, PythonOptions,
};
use crate::code_rule_dsl::scope::flow::Defined;
use crate::code_rule_dsl::scope::usage::{Steps, Usage};
use crate::code_rule_dsl::scope::{
    Access, Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeKind,
    ScopeTable, ecma, php, python,
};
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::sync::Arc;
pub(in crate::code_rule_dsl) mod lang_python;
pub(in crate::code_rule_dsl) mod lang_ecma;
pub(in crate::code_rule_dsl) mod lang_php;

/// A `resolves` or `binds` option some family has no counterpart for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ScopeFeature {
    /// `resolves.this`: JavaScript's global `this`.
    This,
    /// `resolves.sloppyFunctions`.
    SloppyFunctions,
    /// `resolves.directives`: ESLint's `/* global */` comments.
    GlobalComments,
    /// `resolves.declared`.
    Declared,
    /// `resolves.scriptDeclarations`.
    ScriptDeclarations,
    /// `resolves.relative`.
    Relative,
    /// `resolves.seenBefore`: Ruff's `seen_module`.
    SeenBefore,
    /// `resolves.beforeLocal`. ESLint's message runs on from the key (`{path}/beforeLocal reads`).
    BeforeLocal,
    /// `resolves.unresolved` but `true`: whether a star import binds the name.
    StarImports,
    /// `resolves.import` and `resolves.imported`.
    Imports,
    /// `resolves.binding.stopAtGlobal: true`.
    StopAtGlobal,
    /// `resolves.binding.stopAtGlobal: false`: a read through a `global` statement.
    ThroughGlobal,
    /// `occurrence`: a variable's occurrences.
    Occurrence,
    /// `resolves.write`: where ESLint reports an unused variable.
    LastWrite,
    /// `resolves.binding.destructuredValue`.
    DestructuredValue,
    /// A binding filter, by its key (`ScopeRules::REFUSED_FILTERS`).
    Filter(&'static str),
    /// `redefines.every`.
    Every,
    /// `redefines.sameImport`.
    SameImport,
}

/// A family's scope semantics.
pub(crate) trait ScopeRules {
    /// Whether the family builds scope tables: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(test), allow(dead_code))]
    const SUPPORTED: bool;
    /// Whether a read's binding is the one the file's flow reaches it with (PHP's `scope/php.rs`)
    /// rather than a lexical scope's. The family's `resolves` reads the file's names
    /// (`php_names`) and its variables' flow; a read's binding, a name resolved at run time, a
    /// binding's `latest`, `outer` and redefinitions may be unknown.
    const FLOW: bool;
    /// Whether the family reads a stub file (`.pyi`) as Ruff's `is_stub` does.
    const STUBS: bool;
    /// Whether `redefines` reads the one binding a binding shadows (Ruff's `shadowed_bindings`),
    /// rather than its variable's other bindings.
    const REDEFINES_SHADOWED: bool;
    /// The kinds a reference has: what `resolves.global` and `resolves.binding` resolve.
    const REFERENCE_KINDS: &'static [&'static str];
    /// The kinds of a string whose type annotation's names `resolves.unresolved` reads (Python's
    /// string type definitions).
    const ANNOTATION_STRING_KINDS: &'static [&'static str];
    /// The kinds a binding's name node has (`binds`, `resolves.write`).
    const NAME_KINDS: &'static [&'static str];
    /// The kinds of a parenthesized expression, which `binds` and `resolves` read through.
    const PAREN_KINDS: &'static [&'static str];
    /// The kinds of a dotted name, the only thing `resolves.imported` reads (Python's).
    const DOTTED_NAME_KINDS: &'static [&'static str];
    /// The kinds of the attribute chain or path that `resolves.import` follows to an import.
    const ATTRIBUTE_KINDS: &'static [&'static str];
    /// The kinds of `this`, read by `resolves.global` when it lists `this`.
    const THIS_KINDS: &'static [&'static str];
    /// The kinds of a private name, a binding of its class body that `binds` matches too.
    const PRIVATE_NAME_KINDS: &'static [&'static str];
    /// The binding forms a `form` filter names.
    const FORMS: &'static [(&'static str, Form)];
    /// The scopes a `scope` filter names.
    const SCOPES: &'static [(&'static str, ScopeName)];
    /// The binding filters the family refuses, in the order compiling checks them.
    const REFUSED_FILTERS: &'static [&'static str];
    /// Whether the qualified names `resolves.import` lists are paths whose segments `::` joins
    /// (Rust), read by `import_path`, rather than dotted names an attribute chain reads (Python).
    const IMPORT_PATHS: bool;

    /// Visits the file at `root` into `builder`'s table.
    fn visit<D: Doc>(
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        reading: Reading,
    ) -> Result<(), BuildError>;
    /// Completes a visited table, whose references are sorted.
    fn finish<D: Doc>(table: &mut ScopeTable, root: &Node<'_, D>);
    /// The binding `name` resolves to at `reference` (`ScopeTable::resolve`).
    fn resolve<'t>(table: &'t ScopeTable, reference: &Reference, name: &str)
    -> Option<&'t Binding>;
    /// The binding `name` resolves to at `reference` and the value it carries
    /// (`ScopeTable::binding`).
    fn binding<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(&'t Binding, Option<NodeRef>)>;
    /// `binding` under `lookup`, which is not the default (`ScopeTable::binding_by`).
    fn binding_by<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(&'t Binding, Option<NodeRef>)>;
    /// The binding `binding_by` reads, by its scope and its index among that scope's bindings of
    /// the name.
    fn index(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(u32, usize)>;
    /// Whether a read's binding is not known, though some binding may reach it.
    fn undecided(table: &ScopeTable, reference: &Reference) -> bool;
    /// A function's bare annotation a read of `name` at `reference` stops at, before any binding:
    /// an unbound local (Ruff's `annotation_binding_id`).
    fn unbound_annotation(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(u32, usize)>;
    /// Whether a read of `name` at `reference`, which no binding resolves, is unresolved as
    /// `mode` asks (Ruff's F821 and F405, ESLint's no-undef): no builtin, configured global or
    /// implicit name declares it. `python` and `options` are the query's Ruff and ESLint
    /// settings; `directives` the file's `/* global */` comments, where honored.
    fn unresolved(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        mode: Unresolved,
        python: &PythonOptions,
        options: Option<&LanguageOptions>,
        directives: Option<&Directives>,
    ) -> bool;
    /// ESLint's settings declaring the family's globals: `options`, the query's, else ESLint's
    /// defaults (the latest `ecmaVersion`, each file's source type); `None` for a family they do
    /// not configure.
    fn eslint_options(options: Option<&Arc<LanguageOptions>>) -> Option<Arc<LanguageOptions>>;
    /// Ruff's settings declaring the family's builtins: `python`, the query's, else Ruff's
    /// default version; `None` for a family they do not configure.
    fn ruff_options(python: Option<&Arc<PythonOptions>>) -> Option<Arc<PythonOptions>>;
    /// `ScopeTable::unbound_locally`.
    #[cfg(test)]
    fn unbound_locally(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        builtin: bool,
    ) -> bool;
    /// The usage index of `table`, whose file's root is `root`.
    fn usage<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Usage, BuildError>;
    /// The module of a file at `path` no host named and no package holds.
    fn unpackaged_module(path: &str) -> Option<ModulePath>;
    /// The path the read at `node` names through the file's imports (`resolves.import` where
    /// `IMPORT_PATHS`); `file` is the host's path of the file, which places it in its crate.
    fn import_path<D: Doc>(
        table: &ScopeTable,
        node: &Node<'_, D>,
        file: Option<&str>,
    ) -> Option<String>;
    /// Whether the family reads `feature`.
    fn feature(feature: ScopeFeature) -> Result<(), Refusal>;

    /// The family's own name table for the file at `root` (PHP's `PhpNames`), built within `cap` visited nodes:
    /// `None` where the family has none.
    fn names<D: Doc>(
        root: Node<'_, D>,
        cap: usize,
        deadline: &dyn Fn() -> bool,
    ) -> Option<Result<crate::code_rule_dsl::scope::NameTable, BuildError>> {
        let _ = (root, cap, deadline);
        None
    }

    /// The name table `table`'s build read, where it holds one.
    fn table_names(table: &ScopeTable) -> Option<crate::code_rule_dsl::scope::NameTable> {
        let _ = table;
        None
    }

    /// The value node of the constant `node` fetches, by the file's `names` (`value.follow`).
    fn constant<D: Doc>(names: &crate::code_rule_dsl::scope::NameTable, node: &Node<'_, D>) -> Option<NodeRef> {
        let _ = (names, node);
        None
    }

    /// Whether `resolves` holds of `node` by the family's own reading (PHP's names and variable flow); `None`
    /// where the shared reading decides.
    fn resolves<D: Doc>(resolves: &crate::code_rule_dsl::matcher::Resolves, node: &Node<'_, D>) -> Option<bool> {
        let _ = (resolves, node);
        None
    }
}

/// The refusal reporting `message`.
fn refused(message: impl Into<Cow<'static, str>>) -> Result<(), Refusal> {
    Err(Refusal::Refused(message.into()))
}

/// The forms Python and TypeScript bind names in.
const LEXICAL_FORMS: &[(&str, Form)] = &[
    ("assignment", Form::Assignment),
    ("annotated", Form::Annotated),
    ("annotation", Form::Annotation),
    ("augmented", Form::Augmented),
    ("named", Form::Named),
    ("for", Form::For),
    ("with", Form::With),
    ("except", Form::Except),
    ("import", Form::Import),
    ("definition", Form::Definition),
    ("parameter", Form::Parameter),
    ("global", Form::Global),
    ("match", Form::Match),
    ("typeParameter", Form::TypeParameter),
    ("typeAlias", Form::TypeAlias),
    ("implicit", Form::Implicit),
    ("deletion", Form::Deletion),
    ("field", Form::Field),
    ("method", Form::Method),
    ("accessor", Form::Accessor),
];
