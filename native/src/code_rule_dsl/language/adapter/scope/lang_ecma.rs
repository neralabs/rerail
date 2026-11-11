//! Moved from `scope.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::scope::usage::lang_ecma::EcmaUsage as _;

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
    const PAREN_KINDS: &'static [&'static str] = &["parenthesized_expression"];
    const DOTTED_NAME_KINDS: &'static [&'static str] = &[];
    /// The ECMA grammars have no such kind; Vue's own tree, which reads this family's scope, does.
    const ATTRIBUTE_KINDS: &'static [&'static str] = &["attribute"];
    const THIS_KINDS: &'static [&'static str] = &["this"];
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
    const IMPORT_PATHS: bool = false;

    /// Each region of the document (`regions`), read as its dialect: a TypeScript file's root,
    /// or the program an embedding file's tape holds.
    fn visit<D: Doc>(
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        _: Reading,
    ) -> Result<(), BuildError> {
        ecma::visit(builder, root)
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
            && !options.is_some_and(|options| {
                options.declares_in(
                    name,
                    reference.spaces,
                    directives,
                    file_source_type_for(options),
                )
            })
    }

    fn eslint_options(options: Option<&Arc<LanguageOptions>>) -> Option<Arc<LanguageOptions>> {
        Some(
            options.cloned().unwrap_or_else(|| {
                Arc::new(LanguageOptions::new(EsGlobals::get().latest, None, []))
            }),
        )
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

    fn import_path<D: Doc>(_: &ScopeTable, _: &Node<'_, D>, _: Option<&str>) -> Option<String> {
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
