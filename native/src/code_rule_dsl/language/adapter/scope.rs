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

#[cfg(test)]
pub(super) mod kit {
    use super::{Adapter, ScopeFeature, ScopeRules};
    use crate::code_rule_dsl::language::adapter::Refusal;
    use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
    use crate::code_rule_dsl::matcher::Unresolved;
    use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
    use crate::code_rule_dsl::scope::{Form, ScopeTable};
    use crate::code_rule_dsl::tape::TapeDoc;
    use ast_grep_core::{AstGrep, Doc};
    use std::sync::Arc;

    /// What one family's scope rules answer.
    pub(crate) struct Expect {
        pub(crate) flow: bool,
        /// The ESLint `ecmaVersion` and source type (`None`: each file's), and the Ruff version
        /// (its minor), the family's default settings declare its globals by.
        pub(crate) declared: (Option<(u16, Option<SourceType>)>, Option<u8>),
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

    /// The document a file of `source` in `dialect` holds that its family's scope reads (the
    /// program an embedding file embeds), with its dialect and the range of the last `name` in its
    /// text.
    fn read(
        source: &str,
        dialect: GrammarDialect,
        name: &str,
    ) -> (AstGrep<TapeDoc>, GrammarDialect, std::ops::Range<usize>) {
        let doc = super::super::kit::semantic(source, dialect);
        let read = *doc.root().get_doc().get_lang();
        let start = doc
            .root()
            .get_doc()
            .get_source()
            .rfind(name)
            .expect("the name is read");
        (doc, read, start..start + name.len())
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
        let options = Arc::new(LanguageOptions::new(5, Some(SourceType::Script), []));
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
            let (doc, read, range) = read(source, dialect, name);
            let table =
                ScopeTable::build(doc.root(), read, usize::MAX, &|| false).expect("table builds");
            let reference = table.reference(&range).expect("a reference");
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
        let (doc, read, range) = read(source, dialect, name);
        let table =
            ScopeTable::build(doc.root(), read, usize::MAX, &|| false).expect("table builds");
        let reference = table.reference(&range).expect("a reference");
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
