//! Moved from `scope.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

impl ScopeRules for Python {
    const SUPPORTED: bool = true;
    const FLOW: bool = false;
    const STUBS: bool = true;
    const REDEFINES_SHADOWED: bool = true;
    const REFERENCE_KINDS: &'static [&'static str] = &["identifier"];
    const ANNOTATION_STRING_KINDS: &'static [&'static str] = &["string", "concatenated_string"];
    const NAME_KINDS: &'static [&'static str] = &["identifier", "dotted_name"];
    const PAREN_KINDS: &'static [&'static str] = &["parenthesized_expression"];
    const DOTTED_NAME_KINDS: &'static [&'static str] = &["dotted_name"];
    const ATTRIBUTE_KINDS: &'static [&'static str] = &["attribute"];
    const THIS_KINDS: &'static [&'static str] = &[];
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
    const IMPORT_PATHS: bool = false;

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

    fn import_path<D: Doc>(_: &ScopeTable, _: &Node<'_, D>, _: Option<&str>) -> Option<String> {
        None
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
