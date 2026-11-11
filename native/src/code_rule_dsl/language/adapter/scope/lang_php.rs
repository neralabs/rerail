//! Moved from `scope.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// The verdict of a family whose variables are PHP's, named `label`, on `feature`;
/// `last_write` and `same_import` are its messages for those two.
pub(in crate::code_rule_dsl) fn php_feature(
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
    const PAREN_KINDS: &'static [&'static str] = &["parenthesized_expression"];
    /// PHP's `resolves` reads the file's names and flow (`FLOW`), not these three.
    const DOTTED_NAME_KINDS: &'static [&'static str] = &[];
    const ATTRIBUTE_KINDS: &'static [&'static str] = &[];
    const THIS_KINDS: &'static [&'static str] = &[];
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
    const IMPORT_PATHS: bool = false;

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
        let read = table.flow.as_deref()?.read(reference.start)?;
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
                .flow
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

    fn import_path<D: Doc>(_: &ScopeTable, _: &Node<'_, D>, _: Option<&str>) -> Option<String> {
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

    fn names<D: Doc>(
        root: Node<'_, D>,
        cap: usize,
        deadline: &dyn Fn() -> bool,
    ) -> Option<Result<crate::code_rule_dsl::scope::NameTable, BuildError>> {
        Some(
            crate::code_rule_dsl::scope::php_names::PhpNames::build(root, cap, deadline)
                .map(|names| Arc::new(names) as crate::code_rule_dsl::scope::NameTable),
        )
    }

    fn table_names(table: &ScopeTable) -> Option<crate::code_rule_dsl::scope::NameTable> {
        let flow = table.flow.as_deref().and_then(crate::code_rule_dsl::scope::php::Flow::of)?;
        Some(Arc::clone(&flow.names) as crate::code_rule_dsl::scope::NameTable)
    }

    fn constant<D: Doc>(names: &crate::code_rule_dsl::scope::NameTable, node: &Node<'_, D>) -> Option<NodeRef> {
        crate::code_rule_dsl::scope::php_names::PhpNames::of(names)?.constant_value(node)
    }

    fn resolves<D: Doc>(resolves: &crate::code_rule_dsl::matcher::Resolves, node: &Node<'_, D>) -> Option<bool> {
        Some(crate::code_rule_dsl::matcher::resolve_php::holds(resolves, node))
    }
}
