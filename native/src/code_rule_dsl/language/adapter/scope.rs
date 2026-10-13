//! Scope tables, bindings and usage (`resolves`, `binds`): `Semantic::Scope`.

use super::{Adapter, Blade, Ecma, Php, Python, Refusal};
use crate::code_rule_dsl::language::SourceType;
use crate::code_rule_dsl::matcher::{ScopeName, Unresolved, file_module};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::{
    DEFAULT_PYTHON_MINOR, EsGlobals, LanguageOptions, PythonOptions,
};
use crate::code_rule_dsl::scope::php::Defined;
use crate::code_rule_dsl::scope::usage::{Steps, Usage};
use crate::code_rule_dsl::scope::{
    Access, Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeKind,
    ScopeTable, ecma, php, python,
};
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::sync::Arc;

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
    /// The kinds of a private name, a binding of its class body that `binds` matches too.
    const PRIVATE_NAME_KINDS: &'static [&'static str];
    /// The binding forms a `form` filter names.
    const FORMS: &'static [(&'static str, Form)];
    /// The scopes a `scope` filter names.
    const SCOPES: &'static [(&'static str, ScopeName)];
    /// The binding filters the family refuses, in the order compiling checks them.
    const REFUSED_FILTERS: &'static [&'static str];

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
    /// defaults (the latest `ecmaVersion`, a module); `None` for a family they do not configure.
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
    /// Whether the family reads `feature`.
    fn feature(feature: ScopeFeature) -> Result<(), Refusal>;
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

impl ScopeRules for Python {
    const SUPPORTED: bool = true;
    const FLOW: bool = false;
    const STUBS: bool = true;
    const REDEFINES_SHADOWED: bool = true;
    const REFERENCE_KINDS: &'static [&'static str] = &["identifier"];
    const ANNOTATION_STRING_KINDS: &'static [&'static str] = &["string", "concatenated_string"];
    const NAME_KINDS: &'static [&'static str] = &["identifier", "dotted_name"];
    const PRIVATE_NAME_KINDS: &'static [&'static str] = &[];
    const FORMS: &'static [(&'static str, Form)] = LEXICAL_FORMS;
    const SCOPES: &'static [(&'static str, ScopeName)] = &[
        ("module", ScopeName::Kind(ScopeKind::Module)),
        ("class", ScopeName::Kind(ScopeKind::Class)),
        ("function", ScopeName::Kind(ScopeKind::Function)),
        ("lambda", ScopeName::Kind(ScopeKind::Lambda)),
        ("comprehension", ScopeName::Kind(ScopeKind::Comprehension)),
        ("typeParameters", ScopeName::Kind(ScopeKind::TypeParams)),
    ];
    const REFUSED_FILTERS: &'static [&'static str] = &["directives", "builtin"];

    fn visit<D: Doc>(
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        reading: Reading,
    ) -> Result<(), BuildError> {
        python::visit(builder, root, reading)
    }

    fn finish<D: Doc>(table: &mut ScopeTable, root: &Node<'_, D>) {
        python::finish(table, root);
    }

    fn resolve<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<&'t Binding> {
        if reference.access == Access::Del {
            return table.resolve_del(reference, name);
        }
        table.resolve_python(reference, name)
    }

    fn binding<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        let binding = Self::resolve(table, reference, name)?;
        (binding.shadows() && !binding.is_deletion()).then_some((binding, binding.site.value))
    }

    fn binding_by<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        let found = table.python_index_by(reference, name, lookup)?;
        let binding = table.binding_at(found, name);
        Some((binding, binding.site.value))
    }

    fn index(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(u32, usize)> {
        if lookup == Lookup::default() {
            return table.resolve_python_index(reference, name);
        }
        table.python_index_by(reference, name, lookup)
    }

    fn undecided(_: &ScopeTable, _: &Reference) -> bool {
        false
    }

    fn unbound_annotation(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(u32, usize)> {
        table.walk(reference, name, true).unbound
    }

    fn unresolved(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        mode: Unresolved,
        python: &PythonOptions,
        _: Option<&LanguageOptions>,
        _: Option<&Directives>,
    ) -> bool {
        // Ruff stops at an `except` name its handler unbound, as at a deletion, unless the name
        // was a builtin when the handler began (`UnboundException(Some(builtin))`): a binding
        // before the handler is found by the walk, so here only the builtin is left.
        if table.unbound_exception(reference, name) && !python.declares(name) {
            return mode == Unresolved::Unbound;
        }
        let init = file_module().is_some_and(|module| module.last() == "__init__");
        !python.declares(name)
            && !table.implicit(reference, name, init)
            && table.star_visible(reference) == (mode == Unresolved::Star)
    }

    fn eslint_options(_: Option<&Arc<LanguageOptions>>) -> Option<Arc<LanguageOptions>> {
        None
    }

    fn ruff_options(python: Option<&Arc<PythonOptions>>) -> Option<Arc<PythonOptions>> {
        Some(
            python
                .cloned()
                .unwrap_or_else(|| Arc::new(PythonOptions::new(DEFAULT_PYTHON_MINOR))),
        )
    }

    #[cfg(test)]
    fn unbound_locally(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        builtin: bool,
    ) -> bool {
        table.ruff_unbound_locally(reference, name, builtin)
    }

    fn usage<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Usage, BuildError> {
        Usage::python(table, root, steps)
    }

    /// Ruff names a file outside any package by its stem.
    fn unpackaged_module(path: &str) -> Option<ModulePath> {
        ModulePath::stem(path)
    }

    fn feature(feature: ScopeFeature) -> Result<(), Refusal> {
        match feature {
            ScopeFeature::This => refused("`this` is TypeScript's; Python has none"),
            ScopeFeature::GlobalComments => {
                refused("/* global */ comments are ESLint's; Python has none")
            }
            ScopeFeature::ScriptDeclarations => refused("scripts are ESLint's; Python has none"),
            ScopeFeature::Occurrence => {
                refused("a variable's occurrences are TypeScript's; Python has none")
            }
            ScopeFeature::LastWrite => {
                refused("where ESLint reports a variable is TypeScript's; Python has none")
            }
            ScopeFeature::DestructuredValue => refused(
                "destructured values are TypeScript's; a Python unpacked name keeps its element",
            ),
            ScopeFeature::Filter("directives") => {
                refused("/* exported */ comments are ESLint's; Python has none")
            }
            ScopeFeature::Filter("builtin") => {
                refused("configured globals are ESLint's; Python has none")
            }
            ScopeFeature::Every => refused(
                "Python's redefines reads the one binding shadowed; `every` is TypeScript's",
            ),
            ScopeFeature::SloppyFunctions
            | ScopeFeature::Declared
            | ScopeFeature::Relative
            | ScopeFeature::SeenBefore
            | ScopeFeature::BeforeLocal
            | ScopeFeature::StarImports
            | ScopeFeature::Imports
            | ScopeFeature::StopAtGlobal
            | ScopeFeature::ThroughGlobal
            | ScopeFeature::Filter(_)
            | ScopeFeature::SameImport => Ok(()),
        }
    }
}

impl ScopeRules for Ecma {
    const SUPPORTED: bool = true;
    const FLOW: bool = false;
    const STUBS: bool = false;
    const REDEFINES_SHADOWED: bool = false;
    const REFERENCE_KINDS: &'static [&'static str] = &[
        "identifier",
        "type_identifier",
        "shorthand_property_identifier",
        "shorthand_property_identifier_pattern",
        "undefined",
    ];
    const ANNOTATION_STRING_KINDS: &'static [&'static str] = &[];
    const NAME_KINDS: &'static [&'static str] = &[
        "identifier",
        "type_identifier",
        "property_identifier",
        "shorthand_property_identifier_pattern",
        "undefined",
    ];
    const PRIVATE_NAME_KINDS: &'static [&'static str] = &["private_property_identifier"];
    const FORMS: &'static [(&'static str, Form)] = LEXICAL_FORMS;
    /// eslint-scope's names: `global` is a script's top level, `module` a module's or CommonJS
    /// file's; `functionName` holds a named function expression's own name.
    const SCOPES: &'static [(&'static str, ScopeName)] = &[
        ("global", ScopeName::Global),
        ("module", ScopeName::Kind(ScopeKind::Module)),
        ("function", ScopeName::Kind(ScopeKind::Function)),
        ("class", ScopeName::Kind(ScopeKind::Class)),
        ("block", ScopeName::Kind(ScopeKind::Block)),
        ("namespace", ScopeName::Kind(ScopeKind::Namespace)),
        ("typeParameters", ScopeName::Kind(ScopeKind::TypeParams)),
        ("functionName", ScopeName::Kind(ScopeKind::FunctionName)),
    ];
    const REFUSED_FILTERS: &'static [&'static str] = &["latest", "outer"];

    fn visit<D: Doc>(
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        _: Reading,
    ) -> Result<(), BuildError> {
        ecma::visit_program(builder, root)
    }

    fn finish<D: Doc>(table: &mut ScopeTable, root: &Node<'_, D>) {
        ecma::finish(table, root);
    }

    fn resolve<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<&'t Binding> {
        table.resolve_lexical(reference, name)
    }

    /// A value where the name has a single definition, a declarator of the plain name with an
    /// initializer, and no reference writes it (eslint-utils' `isEffectivelyConst`).
    fn binding<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        let (scope, binding) = table.resolve_lexical_scope(reference, name)?;
        let definitions = table.scopes[scope as usize]
            .bindings
            .get(name)
            .map_or(0, Vec::len);
        let value = binding
            .site
            .value
            .filter(|_| definitions == 1 && binding.writes == 0);
        Some((binding, value))
    }

    fn binding_by<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
        _: Lookup,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        let (scope, _) = table.resolve_lexical_scope(reference, name)?;
        let single = table.scopes[scope as usize]
            .bindings
            .get(name)
            .is_some_and(|bindings| bindings.len() == 1);
        Self::binding(table, reference, name).filter(|_| single)
    }

    fn index(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        _: Lookup,
    ) -> Option<(u32, usize)> {
        table.resolve_lexical_index(reference, name)
    }

    fn undecided(_: &ScopeTable, _: &Reference) -> bool {
        false
    }

    fn unbound_annotation(_: &ScopeTable, _: &Reference, _: &str) -> Option<(u32, usize)> {
        None
    }

    fn unresolved(
        _: &ScopeTable,
        reference: &Reference,
        name: &str,
        mode: Unresolved,
        _: &PythonOptions,
        options: Option<&LanguageOptions>,
        directives: Option<&Directives>,
    ) -> bool {
        mode == Unresolved::Unbound
            && !options
                .is_some_and(|options| options.declares_in(name, reference.spaces, directives))
    }

    fn eslint_options(options: Option<&Arc<LanguageOptions>>) -> Option<Arc<LanguageOptions>> {
        Some(options.cloned().unwrap_or_else(|| {
            Arc::new(LanguageOptions::new(
                EsGlobals::get().latest,
                SourceType::Module,
                [],
            ))
        }))
    }

    fn ruff_options(_: Option<&Arc<PythonOptions>>) -> Option<Arc<PythonOptions>> {
        None
    }

    #[cfg(test)]
    fn unbound_locally(_: &ScopeTable, _: &Reference, _: &str, _: bool) -> bool {
        false
    }

    fn usage<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Usage, BuildError> {
        Usage::ecma(table, root, steps)
    }

    fn unpackaged_module(_: &str) -> Option<ModulePath> {
        None
    }

    fn feature(feature: ScopeFeature) -> Result<(), Refusal> {
        match feature {
            ScopeFeature::BeforeLocal => {
                refused("reads Python's binding order; TypeScript has none")
            }
            ScopeFeature::StarImports => refused(
                "\"star\" asks whether a Python star import binds the name; TypeScript has none",
            ),
            ScopeFeature::Imports => refused(
                "import resolution is Python-only for now; TypeScript rules can use `global`",
            ),
            ScopeFeature::StopAtGlobal => {
                refused("`global` statements are Python's; TypeScript has none")
            }
            ScopeFeature::Filter(key @ ("latest" | "outer")) => refused(format!(
                "`{key}` reads Python's binding order and declarations; TypeScript has none"
            )),
            ScopeFeature::SameImport => {
                refused("qualified import names are Python's; TypeScript has none")
            }
            ScopeFeature::This
            | ScopeFeature::SloppyFunctions
            | ScopeFeature::GlobalComments
            | ScopeFeature::Declared
            | ScopeFeature::ScriptDeclarations
            | ScopeFeature::Relative
            | ScopeFeature::SeenBefore
            | ScopeFeature::ThroughGlobal
            | ScopeFeature::Occurrence
            | ScopeFeature::LastWrite
            | ScopeFeature::DestructuredValue
            | ScopeFeature::Filter(_)
            | ScopeFeature::Every => Ok(()),
        }
    }
}

/// The verdict of a family whose variables are PHP's, named `label`, on `feature`;
/// `last_write` and `same_import` are its messages for those two.
fn php_feature(
    feature: ScopeFeature,
    label: &'static str,
    last_write: &'static str,
    same_import: &'static str,
) -> Result<(), Refusal> {
    let why = match feature {
        ScopeFeature::This => {
            "`this` asks whether JavaScript's `this` is the global object; PHP's `$this` never is: outside an object context it is undefined, which `unresolved: true` reports"
        }
        ScopeFeature::SloppyFunctions => {
            "sloppy-mode functions are JavaScript's; PHP has no strict and sloppy modes"
        }
        ScopeFeature::GlobalComments => {
            "/* global */ comments are ESLint's; PHP's superglobals are fixed, and a function declares a global with a `global` statement, which `binding` reads"
        }
        ScopeFeature::Declared => {
            "declared globals come from ESLint's and Ruff's configuration; PHP's superglobals are always declared, and no languageOptions configure others"
        }
        ScopeFeature::ScriptDeclarations => {
            "scripts are ESLint's; a PHP file has no script mode, and a superglobal is never a file's declaration"
        }
        ScopeFeature::Relative => {
            "a `use` statement names what it imports fully qualified (`use namespace\\A;` does not parse); `import` resolves a `namespace\\A` reference"
        }
        ScopeFeature::SeenBefore => {
            "Ruff's `seen_module` asks whether a module was imported before; a PHP `use` only aliases a name for the rest of its namespace and loads nothing"
        }
        ScopeFeature::BeforeLocal => {
            "a PHP function sees no variable of an enclosing scope, so a read before the function binds the name is undefined, which `unresolved: true` reports"
        }
        ScopeFeature::StarImports => {
            return refused(format!("{label} policies ask `unresolved: true`"));
        }
        // PHP's flow already stops at a `global` statement: the read's binding is the
        // statement's. Which top-level binding the global variable holds depends on when the
        // function runs, so reading through it is refused.
        ScopeFeature::ThroughGlobal => {
            return refused(format!(
                "a {label} read after `global $x` reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so {label} has no lookup through it"
            ));
        }
        ScopeFeature::LastWrite => return refused(last_write),
        ScopeFeature::SameImport => return refused(same_import),
        ScopeFeature::Filter(key) => {
            let what = match key {
                "exported" => {
                    "Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports)"
                }
                "builtin" => {
                    "ESLint's configured globals; PHP's superglobals are no file's bindings (an assignment to `$_GET` writes the superglobal)"
                }
                "directives" => "ESLint's /* exported */ comments; PHP exports no variables",
                _ => return Ok(()),
            };
            return refused(format!("`{key}` reads {what}; {label} has none"));
        }
        ScopeFeature::Imports
        | ScopeFeature::StopAtGlobal
        | ScopeFeature::Occurrence
        | ScopeFeature::DestructuredValue
        | ScopeFeature::Every => return Ok(()),
    };
    refused(format!("{why}; {label} has none"))
}

impl ScopeRules for Php {
    const SUPPORTED: bool = true;
    const FLOW: bool = true;
    const STUBS: bool = false;
    const REDEFINES_SHADOWED: bool = false;
    const REFERENCE_KINDS: &'static [&'static str] = &["variable_name"];
    const ANNOTATION_STRING_KINDS: &'static [&'static str] = &[];
    const NAME_KINDS: &'static [&'static str] = &["variable_name"];
    const PRIVATE_NAME_KINDS: &'static [&'static str] = &[];
    const FORMS: &'static [(&'static str, Form)] = &[
        ("assignment", Form::Assignment),
        ("augmented", Form::Augmented),
        ("for", Form::For),
        ("except", Form::Except),
        ("parameter", Form::Parameter),
        ("global", Form::Global),
        ("static", Form::Static),
    ];
    /// The file's top level (`global`), a function or method, a closure or arrow function.
    const SCOPES: &'static [(&'static str, ScopeName)] = &[
        ("global", ScopeName::Kind(ScopeKind::Module)),
        ("function", ScopeName::Kind(ScopeKind::Function)),
        ("closure", ScopeName::Kind(ScopeKind::Lambda)),
    ];
    const REFUSED_FILTERS: &'static [&'static str] = &["exported", "builtin", "directives"];

    fn visit<D: Doc>(
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        _: Reading,
    ) -> Result<(), BuildError> {
        php::visit_program(builder, root)
    }

    /// The flow resolved every read as it went.
    fn finish<D: Doc>(_: &mut ScopeTable, _: &Node<'_, D>) {}

    fn resolve<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<&'t Binding> {
        let (scope, index) = table.php_index(reference)?;
        Some(&table.scopes[scope as usize].bindings.get(name)?[index])
    }

    /// A value where one binding reaches the read on every path, it assigns a value, and the
    /// variable is no reference whose value another name may change.
    fn binding<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        let binding = Self::resolve(table, reference, name)?;
        let read = table.php.as_deref()?.read(reference.start)?;
        Some((binding, binding.site.value.filter(|_| !read.aliased)))
    }

    /// A read after `global $x` reads that statement's binding: PHP's flow stops there.
    fn binding_by<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        let (scope, _) = table.php_index(reference)?;
        let single = table.scopes[scope as usize]
            .bindings
            .get(name)
            .is_some_and(|bindings| bindings.len() == 1);
        Self::binding(table, reference, name).filter(|_| !lookup.only || single)
    }

    fn index(
        table: &ScopeTable,
        reference: &Reference,
        _: &str,
        _: Lookup,
    ) -> Option<(u32, usize)> {
        table.php_index(reference)
    }

    /// Several bindings may reach the read, a path may reach none, or PHP's dynamic symbol table
    /// may decide. A read no binding reaches is decided (it resolves to none).
    fn undecided(table: &ScopeTable, reference: &Reference) -> bool {
        table.php_index(reference).is_none()
            && table
                .php
                .as_deref()
                .and_then(|flow| flow.read(reference.start))
                .is_some_and(|read| !read.bindings.is_empty() || read.defined == Defined::Unknown)
    }

    fn unbound_annotation(_: &ScopeTable, _: &Reference, _: &str) -> Option<(u32, usize)> {
        None
    }

    fn unresolved(
        _: &ScopeTable,
        _: &Reference,
        _: &str,
        _: Unresolved,
        _: &PythonOptions,
        _: Option<&LanguageOptions>,
        _: Option<&Directives>,
    ) -> bool {
        unreachable!("a variable's flow resolves PHP's superglobals (`php_variable`)")
    }

    fn eslint_options(_: Option<&Arc<LanguageOptions>>) -> Option<Arc<LanguageOptions>> {
        None
    }

    fn ruff_options(_: Option<&Arc<PythonOptions>>) -> Option<Arc<PythonOptions>> {
        None
    }

    #[cfg(test)]
    fn unbound_locally(_: &ScopeTable, _: &Reference, _: &str, _: bool) -> bool {
        false
    }

    fn usage<D: Doc>(
        table: &ScopeTable,
        _: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Usage, BuildError> {
        Usage::php(table, steps)
    }

    fn unpackaged_module(_: &str) -> Option<ModulePath> {
        None
    }

    fn feature(feature: ScopeFeature) -> Result<(), Refusal> {
        php_feature(
            feature,
            <Self as Adapter>::LABEL,
            "ESLint reports an unused variable once, at its last write; php's usage oracle, Psalm, reports each unused assignment, which `binds: { used: false }` matches",
            "qualified import names are Python's; a PHP variable is never bound by an import",
        )
    }
}

/// The PHP the template embeds, as the compiled view runs it (`blade::view`): PHP's scope, but
/// for the visit of the view's program.
impl ScopeRules for Blade {
    const SUPPORTED: bool = true;
    const FLOW: bool = <Php as ScopeRules>::FLOW;
    const STUBS: bool = <Php as ScopeRules>::STUBS;
    const REDEFINES_SHADOWED: bool = <Php as ScopeRules>::REDEFINES_SHADOWED;
    const REFERENCE_KINDS: &'static [&'static str] = <Php as ScopeRules>::REFERENCE_KINDS;
    const ANNOTATION_STRING_KINDS: &'static [&'static str] =
        <Php as ScopeRules>::ANNOTATION_STRING_KINDS;
    const NAME_KINDS: &'static [&'static str] = <Php as ScopeRules>::NAME_KINDS;
    const PRIVATE_NAME_KINDS: &'static [&'static str] = <Php as ScopeRules>::PRIVATE_NAME_KINDS;
    const FORMS: &'static [(&'static str, Form)] = <Php as ScopeRules>::FORMS;
    const SCOPES: &'static [(&'static str, ScopeName)] = <Php as ScopeRules>::SCOPES;
    const REFUSED_FILTERS: &'static [&'static str] = <Php as ScopeRules>::REFUSED_FILTERS;

    fn visit<D: Doc>(
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        _: Reading,
    ) -> Result<(), BuildError> {
        for region in <Self as Adapter>::regions(root) {
            php::visit_blade_program(builder, &region.root)?;
        }
        Ok(())
    }

    fn finish<D: Doc>(table: &mut ScopeTable, root: &Node<'_, D>) {
        <Php as ScopeRules>::finish(table, root);
    }

    fn resolve<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<&'t Binding> {
        <Php as ScopeRules>::resolve(table, reference, name)
    }

    fn binding<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        <Php as ScopeRules>::binding(table, reference, name)
    }

    fn binding_by<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        <Php as ScopeRules>::binding_by(table, reference, name, lookup)
    }

    fn index(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(u32, usize)> {
        <Php as ScopeRules>::index(table, reference, name, lookup)
    }

    fn undecided(table: &ScopeTable, reference: &Reference) -> bool {
        <Php as ScopeRules>::undecided(table, reference)
    }

    fn unbound_annotation(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(u32, usize)> {
        <Php as ScopeRules>::unbound_annotation(table, reference, name)
    }

    fn unresolved(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        mode: Unresolved,
        python: &PythonOptions,
        options: Option<&LanguageOptions>,
        directives: Option<&Directives>,
    ) -> bool {
        <Php as ScopeRules>::unresolved(table, reference, name, mode, python, options, directives)
    }

    fn eslint_options(options: Option<&Arc<LanguageOptions>>) -> Option<Arc<LanguageOptions>> {
        <Php as ScopeRules>::eslint_options(options)
    }

    fn ruff_options(python: Option<&Arc<PythonOptions>>) -> Option<Arc<PythonOptions>> {
        <Php as ScopeRules>::ruff_options(python)
    }

    #[cfg(test)]
    fn unbound_locally(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        builtin: bool,
    ) -> bool {
        <Php as ScopeRules>::unbound_locally(table, reference, name, builtin)
    }

    fn usage<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Usage, BuildError> {
        <Php as ScopeRules>::usage(table, root, steps)
    }

    fn unpackaged_module(path: &str) -> Option<ModulePath> {
        <Php as ScopeRules>::unpackaged_module(path)
    }

    fn feature(feature: ScopeFeature) -> Result<(), Refusal> {
        php_feature(
            feature,
            <Self as Adapter>::LABEL,
            "ESLint reports an unused variable once, at its last write; blade reads its embedded PHP, whose usage oracle, Psalm, reports each unused assignment, which `binds: { used: false }` matches",
            "qualified import names are Python's; a blade variable is never bound by an import",
        )
    }
}

#[cfg(test)]
pub(super) mod kit {
    use super::{Adapter, ScopeFeature, ScopeRules};
    use crate::code_rule_dsl::language::adapter::Refusal;
    use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
    use crate::code_rule_dsl::matcher::Unresolved;
    use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
    use crate::code_rule_dsl::scope::{Form, ScopeTable};
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::AstGrep;
    use std::sync::Arc;

    /// What one family's scope rules answer.
    pub(crate) struct Expect {
        pub(crate) flow: bool,
        /// The ESLint `ecmaVersion` and source type, and the Ruff version (its minor), the
        /// family's default settings declare its globals by.
        pub(crate) declared: (Option<(u16, SourceType)>, Option<u8>),
        /// Files, a name each reads last that no binding resolves, and whether that read is
        /// unresolved under the default settings.
        pub(crate) unresolved: &'static [(&'static str, &'static str, bool)],
        pub(crate) stubs: bool,
        pub(crate) redefines_shadowed: bool,
        pub(crate) reference_kinds: &'static [&'static str],
        pub(crate) name_kinds: &'static [&'static str],
        /// A file, a name it reads last, and the form of the binding that read resolves to.
        pub(crate) resolves: (&'static str, &'static str, Option<Form>),
        /// Features the family refuses, with their messages.
        pub(crate) refuses: &'static [(ScopeFeature, &'static str)],
        /// Features the family reads.
        pub(crate) reads: &'static [ScopeFeature],
        /// The module of `dir/mod<suffix>` outside any package.
        pub(crate) unpackaged: Option<&'static str>,
    }

    /// A tape of `source` in `dialect`, as files are read: Blade's from its Forte tree, the rest
    /// from tree-sitter's.
    fn tape(source: &str, dialect: GrammarDialect) -> AstGrep<TapeDoc> {
        let grammar = Grammar::new(dialect).expect("grammar");
        let image = TapeImage::parse(&grammar, source, &|| false)
            .expect("parses")
            .expect("encodes");
        AstGrep::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
                .expect("restores"),
        )
    }

    /// `A`'s scope rules against `expect`, its files in `dialect` with `suffix`.
    pub(crate) fn kit<A: Adapter>(dialect: GrammarDialect, suffix: &str, expect: &Expect) {
        assert_eq!(<A::Scope as ScopeRules>::FLOW, expect.flow);
        let eslint = <A::Scope as ScopeRules>::eslint_options(None);
        let ruff = <A::Scope as ScopeRules>::ruff_options(None);
        assert_eq!(
            (
                eslint
                    .as_deref()
                    .map(|options| (options.ecma_version, options.source_type)),
                ruff.as_deref().map(|python| python.minor)
            ),
            expect.declared
        );
        let options = Arc::new(LanguageOptions::new(5, SourceType::Script, []));
        assert_eq!(
            <A::Scope as ScopeRules>::eslint_options(Some(&options))
                .is_some_and(|kept| Arc::ptr_eq(&kept, &options)),
            eslint.is_some()
        );
        let python = Arc::new(PythonOptions::new(12));
        assert_eq!(
            <A::Scope as ScopeRules>::ruff_options(Some(&python))
                .is_some_and(|kept| Arc::ptr_eq(&kept, &python)),
            ruff.is_some()
        );
        let builtins = PythonOptions::new(ruff.as_deref().map_or(0, |python| python.minor));
        for (source, name, unresolved) in expect.unresolved {
            let doc = tape(source, dialect);
            let table = ScopeTable::build(doc.root(), dialect, usize::MAX, &|| false)
                .expect("table builds");
            let start = source.rfind(name).expect("the name is read");
            let reference = table
                .reference(&(start..start + name.len()))
                .expect("a reference");
            assert!(<A::Scope as ScopeRules>::resolve(&table, reference, name).is_none());
            assert_eq!(
                <A::Scope as ScopeRules>::unresolved(
                    &table,
                    reference,
                    name,
                    Unresolved::Unbound,
                    &builtins,
                    eslint.as_deref(),
                    None
                ),
                *unresolved,
                "{source:?}"
            );
        }
        assert_eq!(<A::Scope as ScopeRules>::STUBS, expect.stubs);
        assert_eq!(
            <A::Scope as ScopeRules>::REDEFINES_SHADOWED,
            expect.redefines_shadowed
        );
        assert_eq!(
            <A::Scope as ScopeRules>::REFERENCE_KINDS,
            expect.reference_kinds
        );
        assert_eq!(<A::Scope as ScopeRules>::NAME_KINDS, expect.name_kinds);
        let (source, name, form) = expect.resolves;
        let doc = tape(source, dialect);
        let table =
            ScopeTable::build(doc.root(), dialect, usize::MAX, &|| false).expect("table builds");
        let start = source.rfind(name).expect("the name is read");
        let reference = table
            .reference(&(start..start + name.len()))
            .expect("a reference");
        assert_eq!(
            <A::Scope as ScopeRules>::resolve(&table, reference, name)
                .map(|binding| binding.site.form),
            form,
            "{source:?}"
        );
        for (feature, message) in expect.refuses {
            match <A::Scope as ScopeRules>::feature(*feature) {
                Err(Refusal::Refused(refusal)) => assert_eq!(refusal, *message, "{feature:?}"),
                _ => panic!("{feature:?} is refused"),
            }
        }
        for feature in expect.reads {
            assert!(
                <A::Scope as ScopeRules>::feature(*feature).is_ok(),
                "{feature:?}"
            );
        }
        for key in <A::Scope as ScopeRules>::REFUSED_FILTERS {
            assert!(
                <A::Scope as ScopeRules>::feature(ScopeFeature::Filter(key)).is_err(),
                "{key}"
            );
        }
        assert_eq!(
            <A::Scope as ScopeRules>::unpackaged_module(&format!("dir/mod{suffix}"))
                .map(|module| module.text()),
            expect.unpackaged.map(str::to_owned)
        );
    }
}
