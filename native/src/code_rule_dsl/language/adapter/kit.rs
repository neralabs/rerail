//! The adapter test kit: every family's adapter against known-good literals (`kit`), and the
//! registry's capabilities against the concerns the adapters read. A new family adds one `#[test]`
//! function calling `kit::<Family>(&Expect { .. })`.

use super::value::kit as value_kit;
use super::{
    Adapter, ArgumentRules, CallRules, FormatArgRules, ScopeRules, ValueRules, has_import_targets, is_comment_kind,
    is_paren_kind, parsed_by_forte,
};
use super::{blade::Blade, ecma::Ecma, php::Php, python::Python, rust::Rust, sql::Sql, vue::Vue};

// `with_adapter!` (generated from the registry, `build_languages.rs`): every family's adapter by its module.
include!(concat!(env!("OUT_DIR"), "/with_adapter.rs"));

/// The scope concern's part of the kit: a family's scope rules against expected literals.
mod scope_kit {
    use crate::code_rule_dsl::language::adapter::{Adapter, ScopeFeature, ScopeRules};
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
        let doc = super::semantic(source, dialect);
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
use crate::code_rule_dsl::language::{
    CodeRuleLanguage, Family, GrammarDialect, Semantic, SourceType, Support, kind_ids,
};
use crate::code_rule_dsl::lines::Terminators;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::value::Value;
use ast_grep_core::AstGrep;
use std::borrow::Cow;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

/// A file, and the kind and file range of each of its documents' roots (`embed`): its own tree's,
/// then each embedded program's.
type Documents = (&'static str, &'static [(&'static str, Range<usize>)]);

/// What one family's adapter answers: the expectations `kit` checks. `family`, `label` and
/// `terminators` identify the adapter; `dialect` is the grammar its files are read in; `name` is
/// what the identifier `ﬁle` normalises to; `documents` lists files with the kind and file range of
/// each of their documents' roots; `module` is the family's file suffix and the module of
/// `pkg/mod<suffix>`; and `value`, `calls`, `arguments`, `format_args`, `syntax` and `scope` are the
/// expectations of each concern's kit, whose documents are the ones its semantics read
/// (`semantic`).
struct Expect {
    family: Family,
    label: &'static str,
    terminators: Terminators,
    /// The dialect the family's files are read in.
    dialect: GrammarDialect,
    /// The dialect of the documents the family's semantics read: its files', or (an embedding
    /// family's) the programs they embed, as the kit's sources write them.
    semantic: GrammarDialect,
    /// The name of the identifier `ﬁle`.
    name: &'static str,
    documents: &'static [Documents],
    /// The suffix of the family's files, and the module of `pkg/mod<suffix>` in a package `pkg`.
    module: (&'static str, Option<&'static str>),
    /// Whether the family's files read their nearest `package.json` (`Adapter::PACKAGE_MANIFESTS`),
    /// and whether `file.jsxRuntime` asks for their JSX runtime (`Adapter::JSX_RUNTIMES`).
    package_manifests: (bool, bool),
    value: value_kit::Expect,
    /// What the family's calls answer.
    calls: super::calls::kit::Calls,
    arguments: super::arguments::kit::Expect,
    format_args: super::format_args::kit::Expect,
    syntax: super::syntax::kit::Expect,
    scope: scope_kit::Expect,
}

/// The tape of a file of `source` in `dialect`, as files are read: Blade's from its Forte tree,
/// the rest from tree-sitter's.
fn file(source: &str, dialect: GrammarDialect) -> TapeDoc {
    let grammar = Grammar::new(dialect).expect("grammar");
    let image = TapeImage::parse(&grammar, source, &|| false)
        .expect("parses")
        .expect("encodes");
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).expect("restores")
}

/// The document of a file of `source` in `dialect` its family's semantics read: the program the
/// file embeds, where its dialect embeds one (`embed`), else the file's own tree.
pub(super) fn semantic(source: &str, dialect: GrammarDialect) -> AstGrep<TapeDoc> {
    let host = file(source, dialect);
    let guest = host
        .guests()
        .and_then(|guests| guests.first().map(|guest| guest.doc.clone()));
    AstGrep::doc(guest.unwrap_or(host))
}

/// A directory holding a package `pkg`, removed when dropped.
struct Package(PathBuf);

impl Package {
    fn new(label: &str, suffix: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("rerail-adapter-kit-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pkg")).expect("package directory");
        std::fs::write(root.join("pkg/__init__.py"), "").expect("package marker");
        std::fs::write(root.join(format!("pkg/mod{suffix}")), "").expect("module");
        Self(root)
    }
}

impl Drop for Package {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs the generic suite over family `A` against `expect`: its identity (`family`, `label`,
/// `terminators`), name normalisation, the regions of each document, module paths, the node kinds
/// `rerail-ignore` is read by, and each concern's own kit. A new family adds one `#[test]` function
/// calling `kit::<Family>(&Expect { .. })`.
fn kit<A: Adapter>(expect: &Expect) {
    assert_eq!(A::FAMILY, expect.family);
    assert_eq!(with_adapter!(expect.family, B => B::LABEL), expect.label);
    assert_eq!(A::LABEL, expect.label);
    assert_eq!(A::TERMINATORS, expect.terminators);
    assert_eq!(A::name(Cow::Borrowed("ﬁle")), expect.name);
    assert!(matches!(
        A::name(Cow::Borrowed("file")),
        Cow::Borrowed("file")
    ));
    for (source, documents) in expect.documents {
        let host = file(source, expect.dialect);
        let read = std::iter::once(host.clone())
            .chain(
                host.guests()
                    .iter()
                    .flat_map(|guests| guests.iter().map(|guest| guest.doc.clone())),
            )
            .map(|doc| {
                let root = AstGrep::doc(doc.clone());
                let root = root.root();
                let file = crate::code_rule_dsl::origin::InFile::tape(&doc);
                (root.kind().into_owned(), file.node(&root))
            })
            .collect::<Vec<_>>();
        let expected = documents
            .iter()
            .map(|(kind, range)| ((*kind).to_owned(), range.clone()))
            .collect::<Vec<_>>();
        assert_eq!(read, expected, "{source:?}");
    }
    let (suffix, module) = expect.module;
    let package = Package::new(expect.label, suffix);
    assert_eq!(
        A::module_path(&package.0.join(format!("pkg/mod{suffix}"))),
        module.map(|module| serde_json::Value::String(module.to_owned()))
    );
    // A family names modules from package markers exactly when it derives one for `pkg/mod`.
    assert_eq!(A::PACKAGES, module.is_some());
    assert_eq!(
        (A::PACKAGE_MANIFESTS, A::JSX_RUNTIMES),
        expect.package_manifests
    );
    // A kind one of the family's grammars lacks (a typo, a renamed node) would leave its
    // directives silently unread there: every registry dialect of the family has each kind.
    let dialects = CodeRuleLanguage::ALL
        .into_iter()
        .filter(|language| language.family() == A::FAMILY)
        .flat_map(|language| language.dialects().iter().copied());
    for dialect in dialects {
        for kind in [A::COMMENT_KINDS, A::PREAMBLE_KINDS, A::BODY_KINDS].concat() {
            assert_ne!(dialect.kind_id(kind), 0, "{kind} in {dialect:?}");
        }
    }
    value_kit::check::<A>(&expect.value, |s| semantic(s, expect.dialect));
    super::calls::kit::check::<A>(&expect.calls, expect.dialect);
    super::arguments::kit::check::<A>(&expect.arguments, expect.semantic, |s| {
        semantic(s, expect.dialect)
    });
    super::format_args::kit::check::<A>(&expect.format_args);
    with_adapter!(expect.family, B => super::syntax::kit::check::<B>(&expect.syntax));
    scope_kit::kit::<A>(expect.dialect, expect.module.0, &expect.scope);
}

#[test]
fn python() {
    kit::<Python>(&Expect {
        family: Family::Python,
        label: "python",
        terminators: Terminators::Newlines,
        dialect: GrammarDialect::Python,
        semantic: GrammarDialect::Python,
        name: "file",
        documents: &[("x = 1\n", &[("module", 0..6)])],
        module: (".py", Some("pkg.mod")),
        package_manifests: (false, false),
        value: value_kit::Expect {
            lang: Family::Python,
            string: Value::Str("é".to_owned()),
            numbers: [Value::int(16), Value::int(u64::MAX)],
            decoded: ("x = 0x10\n", "integer", Value::int(16)),
            name_kinds: &["identifier"],
            str_type: Ok(()),
            structure_parts: Ok(()),
            spelled: ("x = 1\n", true),
            name: None,
        },
        calls: super::calls::kit::Calls {
            call_kinds: &["call"],
            function_kinds: &["function_definition"],
            refusal: None,
            seed_language: None,
            subclasses: true,
            summary: ("def f():\n    g()\n", &["f"]),
            unread: Some("pkg.mod"),
            label: ("pkg.C.m", "pkg.C.m"),
            keys: &[("pkg.f", 0, &["pkg.f", "alt.f"]), ("pkg.f", 64, &[])],
        },
        arguments: super::arguments::kit::Expect {
            call_kinds: &["call"],
            features: [
                None,
                None,
                Some(
                    "`estree` reads ESLint's `node.arguments`; use language or syntactic for Python",
                ),
            ],
            passed: ("f(1, b=2)\n", ["1", "2"]),
        },
        format_args: super::format_args::kit::Expect {
            follows_strings: false,
            styles: [
                None,
                None,
                Some(
                    "`printf` reads PHP's printf-family calls; Python's printf formats are style `percent`",
                ),
                Some("`scanf` reads PHP's sscanf and fscanf calls; Python has no scanf formats"),
                Some(
                    "`rust` reads Rust's format macros; Python's format calls are styles `brace` and `percent`",
                ),
            ],
        },
        syntax: super::syntax::kit::Expect {
            dollar_names: true,
            language_options: None,
            compiled_options: Some((r#"{"pythonVersion": "3.12"}"#, None, Some(12))),
            when: &[
                (r#"{"pythonVersion": {"min": "3.10"}}"#, Ok((true, None))),
                (r#"{"pythonVersion": {"max": "3.9"}}"#, Ok((false, None))),
                (
                    r#"{"ecmaVersion": {"min": 2015}}"#,
                    Err("unknown field `ecmaVersion` at /q/when"),
                ),
            ],
            parse: [
                None,
                None,
                None,
                Some("`eslintDirective` parses TypeScript comments"),
                Some(
                    "`printfFormat` parses PHP strings; Python's printf formats are `percentFormat`",
                ),
                Some("`scanfFormat` parses PHP strings; Python has no scanf formats"),
                Some("`phpDoc` parses PHP doc comments"),
                Some("`rustFormat` parses Rust strings"),
            ],
        },
        scope: scope_kit::Expect {
            flow: false,
            declared: (None, Some(10)),
            unresolved: &[
                ("print(undefined)\n", "undefined", true),
                ("x = print\n", "print", false),
            ],
            stubs: true,
            redefines_shadowed: true,
            reference_kinds: &["identifier"],
            name_kinds: &["identifier", "dotted_name"],
            resolves: (
                "x = 1\nprint(x)\n",
                "x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::This,
                    "`this` is TypeScript's; Python has none",
                ),
                (
                    super::ScopeFeature::Every,
                    "Python's redefines reads the one binding shadowed; `every` is TypeScript's",
                ),
            ],
            reads: &[
                super::ScopeFeature::Imports,
                super::ScopeFeature::StopAtGlobal,
            ],
            unpackaged: Some("mod"),
        },
    });
}

#[test]
fn ecma() {
    kit::<Ecma>(&Expect {
        family: Family::Ecma,
        label: "typescript",
        terminators: Terminators::Eslint,
        dialect: GrammarDialect::TypeScript,
        semantic: GrammarDialect::TypeScript,
        name: "ﬁle",
        documents: &[("let x = 1;\n", &[("program", 0..11)])],
        module: (".ts", None),
        package_manifests: (true, true),
        value: value_kit::Expect {
            lang: Family::Ecma,
            string: Value::Utf16(vec![0xe9]),
            numbers: [
                Value::Float(16.0),
                Value::Float(18_446_744_073_709_551_615.0),
            ],
            decoded: ("let x = 0x10;\n", "number", Value::Float(16.0)),
            name_kinds: &["identifier", "undefined"],
            str_type: Err(
                "`str` is not a TypeScript type name; expected one of string, number, bigint, boolean, null, undefined, array, object, regexp",
            ),
            structure_parts: Err(
                "the structural modes compare Python and PHP syntax; use tokens for TypeScript",
            ),
            spelled: ("let x = 1;\n", true),
            name: Some(("foo;\n", "identifier", "foo")),
        },
        calls: super::calls::kit::Calls {
            call_kinds: &[],
            function_kinds: &[],
            refusal: Some("calls follows Python, PHP and Rust calls only"),
            seed_language: None,
            subclasses: false,
            summary: ("function f() { g(); }\n", &[]),
            unread: None,
            label: ("pkg.f", "pkg.f"),
            keys: &[("pkg.f", 0, &[])],
        },
        arguments: super::arguments::kit::Expect {
            call_kinds: &["call_expression", "new_expression"],
            features: [
                Some(
                    "`/` and `*` mark Python's positional-only and keyword-only slots; JavaScript passes arguments by position only",
                ),
                Some(
                    "`syntactic` finds arguments by position and keyword as Ruff's find_argument does; TypeScript has no keywords",
                ),
                None,
            ],
            passed: ("f(1, ...xs);\n", ["1", "?"]),
        },
        format_args: super::format_args::kit::Expect {
            follows_strings: false,
            styles: [Some("formatArgs reads Python, PHP and Rust format calls"); 5],
        },
        syntax: super::syntax::kit::Expect {
            dollar_names: false,
            language_options: None,
            compiled_options: Some((
                r#"{"ecmaVersion": 6, "sourceType": "script"}"#,
                Some(2015),
                None,
            )),
            when: &[
                (r#"{"ecmaVersion": {"min": 2015}}"#, Ok((true, None))),
                (
                    r#"{"sourceType": "script"}"#,
                    Ok((true, Some(&[SourceType::Script]))),
                ),
                (
                    r#"{"pythonVersion": {"min": "3.10"}}"#,
                    Err("unknown field `pythonVersion` at /q/when"),
                ),
            ],
            parse: [
                Some("`percentFormat` parses Python literals"),
                Some("`braceFormat` parses Python literals"),
                Some("`annotation` parses Python literals"),
                None,
                Some("`printfFormat` parses PHP strings"),
                Some("`scanfFormat` parses PHP strings"),
                Some("`phpDoc` parses PHP doc comments"),
                Some("`rustFormat` parses Rust strings"),
            ],
        },
        scope: scope_kit::Expect {
            flow: false,
            declared: (Some((2026, None)), None),
            unresolved: &[
                ("undefinedName;\n", "undefinedName", true),
                ("Math;\n", "Math", false),
            ],
            stubs: false,
            redefines_shadowed: false,
            reference_kinds: &[
                "identifier",
                "type_identifier",
                "shorthand_property_identifier",
                "shorthand_property_identifier_pattern",
                "undefined",
            ],
            name_kinds: &[
                "identifier",
                "type_identifier",
                "property_identifier",
                "shorthand_property_identifier_pattern",
                "undefined",
            ],
            resolves: (
                "let x = 1;\nx;\n",
                "x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::Imports,
                    "import resolution is Python-only for now; TypeScript rules can use `global`",
                ),
                (
                    super::ScopeFeature::Filter("latest"),
                    "`latest` reads Python's binding order and declarations; TypeScript has none",
                ),
            ],
            reads: &[super::ScopeFeature::This, super::ScopeFeature::LastWrite],
            unpackaged: None,
        },
    });
}

#[test]
fn php() {
    kit::<Php>(&Expect {
        family: Family::Php,
        label: "php",
        terminators: Terminators::Newlines,
        dialect: GrammarDialect::Php,
        semantic: GrammarDialect::Php,
        name: "ﬁle",
        documents: &[("<?php $x = 1;\n", &[("program", 0..14)])],
        module: (".php", None),
        package_manifests: (false, false),
        value: value_kit::Expect {
            lang: Family::Php,
            string: Value::PhpStr(vec![0xc3, 0xa9]),
            numbers: [Value::int(16), Value::Float(18_446_744_073_709_551_615.0)],
            decoded: ("<?php $x = 0x10;\n", "integer", Value::int(16)),
            name_kinds: &[
                "variable_name",
                "name",
                "qualified_name",
                "relative_name",
                "class_constant_access_expression",
            ],
            str_type: Err(
                "`str` is not a PHP type name; expected one of int, float, string, bool, null, array",
            ),
            structure_parts: Err(
                "`structure-parts` compares Python's implicitly concatenated string parts; PHP has no implicit string concatenation",
            ),
            spelled: ("<?php $x = 1;\n", true),
            name: Some(("<?php foo();\n", "name", "foo")),
        },
        calls: super::calls::kit::Calls {
            call_kinds: &[
                "function_call_expression",
                "member_call_expression",
                "nullsafe_member_call_expression",
                "scoped_call_expression",
                "object_creation_expression",
            ],
            function_kinds: &[
                "function_definition",
                "method_declaration",
                "anonymous_function",
                "arrow_function",
            ],
            refusal: None,
            seed_language: None,
            subclasses: false,
            summary: ("<?php function f() { g(); }\n", &["\\f"]),
            unread: None,
            label: ("\\app\\c.m", "app\\c::m"),
            keys: &[("\\App\\F", 0, &["\\app\\f"]), ("\\f", 64, &["\\f"])],
        },
        arguments: super::arguments::kit::Expect {
            call_kinds: &[
                "function_call_expression",
                "member_call_expression",
                "nullsafe_member_call_expression",
                "scoped_call_expression",
                "object_creation_expression",
            ],
            features: [
                Some(
                    "`/` and `*` mark Python's positional-only and keyword-only slots; PHP has no positional-only or keyword-only parameters",
                ),
                None,
                Some(
                    "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; php has none (use language or syntactic)",
                ),
            ],
            passed: ("<?php f(1, b: 2);\n", ["1", "2"]),
        },
        format_args: super::format_args::kit::Expect {
            follows_strings: true,
            styles: [
                Some(
                    "`brace` reads Python's `str.format` calls; PHP has no such templates (its printf-family calls are style `printf`)",
                ),
                Some(
                    "`percent` reads Python's `%` formats; PHP's printf-family calls are style `printf`",
                ),
                None,
                None,
                Some(
                    "`rust` reads Rust's format macros; PHP's printf-family calls are style `printf`",
                ),
            ],
        },
        syntax: super::syntax::kit::Expect {
            dollar_names: true,
            language_options: Some("`languageOptions` is not available for php policies"),
            compiled_options: None,
            when: &[
                (r#"{"ecmaVersion": {"min": 2015}}"#, Ok((true, None))),
                (r#"{"sourceType": "script"}"#, Ok((false, None))),
                (
                    r#"{"pythonVersion": {"min": "3.10"}}"#,
                    Err("unknown field `pythonVersion` at /q/when"),
                ),
            ],
            parse: [
                Some(
                    "`percentFormat` parses Python literals; PHP's printf formats are `printfFormat`",
                ),
                Some(
                    "`braceFormat` parses Python literals; PHP has no str.format templates (its printf formats are `printfFormat`)",
                ),
                Some(
                    "`annotation` parses Python literals; PHP holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
                ),
                Some("`eslintDirective` parses TypeScript comments"),
                None,
                None,
                None,
                Some("`rustFormat` parses Rust strings"),
            ],
        },
        scope: scope_kit::Expect {
            flow: true,
            declared: (None, None),
            unresolved: &[],
            stubs: false,
            redefines_shadowed: false,
            reference_kinds: &["variable_name"],
            name_kinds: &["variable_name"],
            resolves: (
                "<?php $x = 1;\necho $x;\n",
                "$x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::StarImports,
                    "php policies ask `unresolved: true`",
                ),
                (
                    super::ScopeFeature::SameImport,
                    "qualified import names are Python's; a PHP variable is never bound by an import",
                ),
            ],
            reads: &[super::ScopeFeature::Imports, super::ScopeFeature::Every],
            unpackaged: None,
        },
    });
}

#[test]
fn blade() {
    kit::<Blade>(&Expect {
        family: Family::Blade,
        label: "blade",
        terminators: Terminators::Newlines,
        dialect: GrammarDialect::Blade,
        semantic: GrammarDialect::Php,
        name: "ﬁle",
        documents: &[
            (
                "<p>{{ $x }}</p>\n",
                &[("document", 0..16), ("program", 3..11)],
            ),
            ("<p>x</p>\n", &[("document", 0..9)]),
        ],
        module: (".blade.php", None),
        package_manifests: (false, false),
        value: value_kit::Expect {
            lang: Family::Php,
            string: Value::PhpStr(vec![0xc3, 0xa9]),
            numbers: [Value::int(16), Value::Float(18_446_744_073_709_551_615.0)],
            decoded: ("<p>{{ 0x10 }}</p>\n", "integer", Value::int(16)),
            name_kinds: &[
                "variable_name",
                "name",
                "qualified_name",
                "relative_name",
                "class_constant_access_expression",
            ],
            str_type: Err(
                "`str` is not a PHP type name; expected one of int, float, string, bool, null, array",
            ),
            structure_parts: Err(
                "`structure-parts` compares Python's implicitly concatenated string parts; PHP has no implicit string concatenation",
            ),
            // `@unless($x)` is `if (!($x))`, whose `!(` the template does not spell.
            spelled: ("@unless($x) a @endunless\n", false),
            name: Some(("<p>{{ foo() }}</p>\n", "name", "foo")),
        },
        calls: super::calls::kit::Calls {
            call_kinds: &[
                "function_call_expression",
                "member_call_expression",
                "nullsafe_member_call_expression",
                "scoped_call_expression",
                "object_creation_expression",
            ],
            function_kinds: &[
                "function_definition",
                "method_declaration",
                "anonymous_function",
                "arrow_function",
            ],
            refusal: None,
            seed_language: Some("php"),
            subclasses: false,
            summary: ("@php function f() { g(); } @endphp\n", &["\\f"]),
            unread: None,
            label: ("\\app\\c.m", "app\\c::m"),
            keys: &[("\\App\\F", 0, &["\\app\\f"]), ("\\f", 64, &["\\f"])],
        },
        arguments: super::arguments::kit::Expect {
            call_kinds: &[
                "function_call_expression",
                "member_call_expression",
                "nullsafe_member_call_expression",
                "scoped_call_expression",
                "object_creation_expression",
            ],
            features: [
                Some(
                    "`/` and `*` mark Python's positional-only and keyword-only slots; PHP has no positional-only or keyword-only parameters",
                ),
                None,
                Some(
                    "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; php has none (use language or syntactic)",
                ),
            ],
            passed: ("{{ f(1, b: 2) }}\n", ["1", "2"]),
        },
        format_args: super::format_args::kit::Expect {
            follows_strings: true,
            styles: [
                Some(
                    "`brace` reads Python's `str.format` calls; PHP has no such templates (its printf-family calls are style `printf`)",
                ),
                Some(
                    "`percent` reads Python's `%` formats; PHP's printf-family calls are style `printf`",
                ),
                None,
                None,
                Some(
                    "`rust` reads Rust's format macros; PHP's printf-family calls are style `printf`",
                ),
            ],
        },
        syntax: super::syntax::kit::Expect {
            dollar_names: false,
            language_options: Some("`languageOptions` is not available for blade policies"),
            compiled_options: None,
            when: &[
                (r#"{"ecmaVersion": {"min": 2015}}"#, Ok((true, None))),
                (r#"{"sourceType": "script"}"#, Ok((false, None))),
                (
                    r#"{"pythonVersion": {"min": "3.10"}}"#,
                    Err("unknown field `pythonVersion` at /q/when"),
                ),
            ],
            // A template's own tree hosts no literal: its PHP is the view's.
            parse: [
                Some(
                    "`percentFormat` parses Python literals; a template's PHP is its embedded program's: read it with `ast.embedded: compiled-view` or `embedded`",
                ),
                Some(
                    "`braceFormat` parses Python literals; a template's PHP is its embedded program's: read it with `ast.embedded: compiled-view` or `embedded`",
                ),
                Some(
                    "`annotation` parses Python literals; a template's PHP is its embedded program's: read it with `ast.embedded: compiled-view` or `embedded`",
                ),
                Some(
                    "`eslintDirective` parses TypeScript comments; a template's PHP is its embedded program's: read it with `ast.embedded: compiled-view` or `embedded`",
                ),
                Some(
                    "`printfFormat` parses PHP strings; a template's PHP is its embedded program's: read it with `ast.embedded: compiled-view` or `embedded`",
                ),
                Some(
                    "`scanfFormat` parses PHP strings; a template's PHP is its embedded program's: read it with `ast.embedded: compiled-view` or `embedded`",
                ),
                Some(
                    "`phpDoc` parses PHP doc comments; a template's PHP is its embedded program's: read it with `ast.embedded: compiled-view` or `embedded`",
                ),
                Some(
                    "`rustFormat` parses Rust strings; a template's PHP is its embedded program's: read it with `ast.embedded: compiled-view` or `embedded`",
                ),
            ],
        },
        scope: scope_kit::Expect {
            flow: true,
            declared: (None, None),
            unresolved: &[],
            stubs: false,
            redefines_shadowed: false,
            reference_kinds: &["variable_name"],
            name_kinds: &["variable_name"],
            resolves: (
                "@php\n    $x = 1;\n@endphp\n{{ $x }}\n",
                "$x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::StarImports,
                    "php policies ask `unresolved: true`",
                ),
                (
                    super::ScopeFeature::SameImport,
                    "qualified import names are Python's; a PHP variable is never bound by an import",
                ),
            ],
            reads: &[super::ScopeFeature::Imports, super::ScopeFeature::Every],
            unpackaged: None,
        },
    });
}

#[test]
fn vue() {
    kit::<Vue>(&Expect {
        family: Family::Vue,
        label: "vue",
        terminators: Terminators::Eslint,
        dialect: GrammarDialect::Vue,
        semantic: GrammarDialect::JavaScript,
        name: "ﬁle",
        documents: &[
            // The program stands for the script's start tag (its `;` marker) to its content's end.
            (
                "<script>\nlet x = 1;\n</script>\n",
                &[("document", 0..30), ("program", 0..19)],
            ),
            (
                "<template><p>{{ x }}</p></template>\n",
                &[("document", 0..36), ("program", 0..35)],
            ),
            ("<style>p {}</style>\n", &[("document", 0..20)]),
            (
                "<script lang=\"coffee\">x = 1</script>\n",
                &[("document", 0..37)],
            ),
        ],
        module: (".vue", None),
        package_manifests: (false, false),
        value: value_kit::Expect {
            lang: Family::Ecma,
            string: Value::Utf16(vec![0xe9]),
            numbers: [
                Value::Float(16.0),
                Value::Float(18_446_744_073_709_551_615.0),
            ],
            decoded: (
                "<script>\nlet x = 0x10;\n</script>\n",
                "number",
                Value::Float(16.0),
            ),
            name_kinds: &["identifier", "undefined"],
            str_type: Err(
                "`str` is not a TypeScript type name; expected one of string, number, bigint, boolean, null, undefined, array, object, regexp",
            ),
            structure_parts: Err(
                "the structural modes compare Python and PHP syntax; use tokens for TypeScript",
            ),
            // `{{ x }}` is `(x);`, whose parentheses and `;` the file does not spell.
            spelled: ("<template>{{ x }}</template>\n", false),
            name: Some(("<template>{{ foo }}</template>\n", "identifier", "foo")),
        },
        calls: super::calls::kit::Calls {
            call_kinds: &[],
            function_kinds: &[],
            refusal: Some("calls follows Python, PHP and Rust calls only"),
            seed_language: None,
            subclasses: false,
            summary: ("<script>\nfunction f() { g(); }\n</script>\n", &[]),
            unread: None,
            label: ("pkg.f", "pkg.f"),
            keys: &[("pkg.f", 0, &[])],
        },
        arguments: super::arguments::kit::Expect {
            call_kinds: &["call_expression", "new_expression"],
            features: [
                Some(
                    "`/` and `*` mark Python's positional-only and keyword-only slots; JavaScript passes arguments by position only",
                ),
                Some(
                    "`syntactic` finds arguments by position and keyword as Ruff's find_argument does; TypeScript has no keywords",
                ),
                None,
            ],
            passed: ("<template>{{ f(1, ...xs) }}</template>\n", ["1", "?"]),
        },
        format_args: super::format_args::kit::Expect {
            follows_strings: false,
            styles: [Some("formatArgs reads Python, PHP and Rust format calls"); 5],
        },
        syntax: super::syntax::kit::Expect {
            dollar_names: false,
            language_options: None,
            compiled_options: Some((
                r#"{"ecmaVersion": 6, "sourceType": "script"}"#,
                Some(2015),
                None,
            )),
            when: &[
                (r#"{"ecmaVersion": {"min": 2015}}"#, Ok((true, None))),
                (r#"{"sourceType": "script"}"#, Ok((false, None))),
                (
                    r#"{"pythonVersion": {"min": "3.10"}}"#,
                    Err("unknown field `pythonVersion` at /q/when"),
                ),
            ],
            // A component's own tree hosts no literal: its scripts are the program's.
            parse: [
                Some(
                    "`percentFormat` parses Python literals; a component's scripts are its embedded program: read them with `ast.embedded: script-block` or `embedded`",
                ),
                Some(
                    "`braceFormat` parses Python literals; a component's scripts are its embedded program: read them with `ast.embedded: script-block` or `embedded`",
                ),
                Some(
                    "`annotation` parses Python literals; a component's scripts are its embedded program: read them with `ast.embedded: script-block` or `embedded`",
                ),
                Some(
                    "`eslintDirective` parses TypeScript comments; a component's scripts are its embedded program: read them with `ast.embedded: script-block` or `embedded`",
                ),
                Some(
                    "`printfFormat` parses PHP strings; a component's scripts are its embedded program: read them with `ast.embedded: script-block` or `embedded`",
                ),
                Some(
                    "`scanfFormat` parses PHP strings; a component's scripts are its embedded program: read them with `ast.embedded: script-block` or `embedded`",
                ),
                Some(
                    "`phpDoc` parses PHP doc comments; a component's scripts are its embedded program: read them with `ast.embedded: script-block` or `embedded`",
                ),
                Some(
                    "`rustFormat` parses Rust strings; a component's scripts are its embedded program: read them with `ast.embedded: script-block` or `embedded`",
                ),
            ],
        },
        scope: scope_kit::Expect {
            flow: false,
            declared: (Some((2026, None)), None),
            unresolved: &[
                (
                    "<template>{{ undefinedName }}</template>\n",
                    "undefinedName",
                    true,
                ),
                ("<script>\nMath;\n</script>\n", "Math", false),
            ],
            stubs: false,
            redefines_shadowed: false,
            reference_kinds: &[
                "identifier",
                "type_identifier",
                "shorthand_property_identifier",
                "shorthand_property_identifier_pattern",
                "undefined",
            ],
            name_kinds: &[
                "identifier",
                "type_identifier",
                "property_identifier",
                "shorthand_property_identifier_pattern",
                "undefined",
            ],
            // A `<script setup>` binding the template reads.
            resolves: (
                "<script setup>\nlet x = 1;\n</script>\n<template>{{ x }}</template>\n",
                "x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::Imports,
                    "import resolution is Python-only for now; TypeScript rules can use `global`",
                ),
                (
                    super::ScopeFeature::Filter("latest"),
                    "`latest` reads Python's binding order and declarations; TypeScript has none",
                ),
            ],
            reads: &[super::ScopeFeature::This, super::ScopeFeature::LastWrite],
            unpackaged: None,
        },
    });
}

#[test]
fn sql() {
    kit::<Sql>(&Expect {
        family: Family::Sql,
        label: "sql",
        terminators: Terminators::Newlines,
        dialect: GrammarDialect::Sql,
        semantic: GrammarDialect::Sql,
        name: "ﬁle",
        documents: &[("SELECT 1;\n", &[("program", 0..10)])],
        module: (".sql", None),
        package_manifests: (false, false),
        value: value_kit::Expect {
            lang: Family::Sql,
            string: Value::Str("é".to_owned()),
            numbers: [Value::int(16), Value::int(u64::MAX)],
            decoded: ("SELECT 0x10;\n", "literal", Value::int(16)),
            name_kinds: &[],
            str_type: Err(
                "a SQL literal's type is its dialect's: PostgreSQL's pg_typeof and SQLite's typeof name different types (1.5 is numeric or real, 'a' unknown or text)",
            ),
            structure_parts: Err(
                "the structural modes compare Python and PHP syntax; use tokens for SQL",
            ),
            spelled: ("SELECT 1;\n", true),
            name: Some(("SELECT Foo;\n", "identifier", "foo")),
        },
        calls: super::calls::kit::Calls {
            call_kinds: &[],
            function_kinds: &[],
            refusal: Some("calls follows Python, PHP and Rust calls only"),
            seed_language: None,
            subclasses: false,
            summary: ("SELECT f();\n", &[]),
            unread: None,
            label: ("pkg.f", "pkg.f"),
            keys: &[("pkg.f", 0, &[])],
        },
        arguments: super::arguments::kit::Expect {
            call_kinds: &["invocation"],
            features: [
                Some(
                    "`/` and `*` mark Python's positional-only and keyword-only slots; a SQL parameter may be passed by position or by name",
                ),
                Some(
                    "`syntactic` finds arguments as Ruff's find_argument does; SQL's are bound as PostgreSQL binds them",
                ),
                Some(
                    "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; SQL has none",
                ),
            ],
            passed: ("SELECT f(1, b => 2);\n", ["1", "2"]),
        },
        format_args: super::format_args::kit::Expect {
            follows_strings: false,
            styles: [Some("formatArgs reads Python, PHP and Rust format calls"); 5],
        },
        syntax: super::syntax::kit::Expect {
            dollar_names: false,
            language_options: Some("`languageOptions` is not available for sql policies"),
            compiled_options: None,
            when: &[
                ("{}", Ok((true, None))),
                (
                    r#"{"ecmaVersion": {"min": 2015}}"#,
                    Err("unknown field `ecmaVersion` at /q/when"),
                ),
            ],
            parse: [
                Some("`percentFormat` parses Python literals"),
                Some("`braceFormat` parses Python literals"),
                Some("`annotation` parses Python literals"),
                Some("`eslintDirective` parses TypeScript comments"),
                Some("`printfFormat` parses PHP strings"),
                Some("`scanfFormat` parses PHP strings"),
                Some("`phpDoc` parses PHP doc comments"),
                Some("`rustFormat` parses Rust strings"),
            ],
        },
        scope: scope_kit::Expect {
            flow: false,
            declared: (None, None),
            unresolved: &[
                ("SELECT t.a FROM x AS u;\n", "t", true),
                ("UPDATE x SET a = t.b;\n", "t", false),
            ],
            stubs: false,
            redefines_shadowed: false,
            reference_kinds: &["identifier"],
            name_kinds: &["identifier"],
            resolves: (
                "SELECT 1 FROM t AS u WHERE u.a = 1;\n",
                "u",
                Some(crate::code_rule_dsl::scope::Form::Relation),
            ),
            refuses: &[
                (
                    super::ScopeFeature::This,
                    "`this` is JavaScript's; SQL has none",
                ),
                (
                    super::ScopeFeature::Filter("latest"),
                    "`latest` reads Python's binding order and declarations; SQL has none",
                ),
            ],
            reads: &[super::ScopeFeature::Every],
            unpackaged: None,
        },
    });
}

#[test]
fn rust() {
    kit::<Rust>(&Expect {
        family: Family::Rust,
        label: "rust",
        terminators: Terminators::Newlines,
        dialect: GrammarDialect::Rust,
        semantic: GrammarDialect::Rust,
        name: "ﬁle",
        documents: &[("fn f() {}\n", &[("source_file", 0..10)])],
        module: (".rs", None),
        package_manifests: (false, false),
        value: value_kit::Expect {
            lang: Family::Rust,
            string: Value::Str("é".to_owned()),
            numbers: [
                Value::RustInt {
                    value: 16.into(),
                    ty: None,
                },
                Value::RustInt {
                    value: u64::MAX.into(),
                    ty: None,
                },
            ],
            decoded: (
                "const X: u8 = 0x10;\n",
                "integer_literal",
                Value::RustInt {
                    value: 16.into(),
                    ty: None,
                },
            ),
            name_kinds: &["identifier"],
            str_type: Err(
                "`str` is not a Rust type name; expected one of bool, char, &str, &[u8], &CStr, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64",
            ),
            structure_parts: Err(
                "the structural modes compare Python and PHP syntax; use tokens for Rust",
            ),
            spelled: ("fn f() {}\n", true),
            name: None,
        },
        calls: super::calls::kit::Calls {
            call_kinds: &["call_expression"],
            function_kinds: &["function_item"],
            refusal: None,
            seed_language: None,
            subclasses: false,
            summary: ("fn f() {\n    g();\n}\n", &["kit/crate::f"]),
            unread: None,
            label: ("crate::a::f", "crate::a::f"),
            keys: &[("crate::f", 0, &["crate::f"])],
        },
        arguments: super::arguments::kit::Expect {
            call_kinds: &["call_expression"],
            features: [
                Some(
                    "`/` and `*` mark Python's positional-only and keyword-only slots; Rust passes arguments by position only",
                ),
                Some(
                    "`syntactic` finds arguments by position and keyword as Ruff's find_argument does; Rust has no keywords",
                ),
                Some(
                    "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; rust has none (use language)",
                ),
            ],
            passed: ("fn main() { f(1, (2)); }\n", ["1", "2"]),
        },
        format_args: super::format_args::kit::Expect {
            follows_strings: false,
            styles: [
                Some(
                    "`brace` reads Python's `str.format` calls; Rust's format macros are style `rust`",
                ),
                Some("`percent` reads Python's `%` formats; Rust's format macros are style `rust`"),
                Some(
                    "`printf` reads PHP's printf-family calls; Rust's format macros are style `rust`",
                ),
                Some("`scanf` reads PHP's sscanf and fscanf calls; Rust has no scanf formats"),
                None,
            ],
        },
        syntax: super::syntax::kit::Expect {
            dollar_names: false,
            language_options: Some("`languageOptions` is not available for rust policies"),
            compiled_options: None,
            when: &[
                (r#"{"ecmaVersion": {"min": 2015}}"#, Ok((true, None))),
                (r#"{"sourceType": "script"}"#, Ok((false, None))),
                (
                    r#"{"pythonVersion": {"min": "3.10"}}"#,
                    Err("unknown field `pythonVersion` at /q/when"),
                ),
            ],
            parse: [
                Some(
                    "`percentFormat` parses Python literals; Rust's format templates are `rustFormat`",
                ),
                Some(
                    "`braceFormat` parses Python literals; Rust's format templates are `rustFormat`",
                ),
                Some("`annotation` parses Python literals"),
                Some("`eslintDirective` parses TypeScript comments"),
                Some("`printfFormat` parses PHP strings; Rust's format templates are `rustFormat`"),
                Some("`scanfFormat` parses PHP strings; Rust has no scanf formats"),
                Some("`phpDoc` parses PHP doc comments"),
                None,
            ],
        },
        scope: scope_kit::Expect {
            flow: false,
            declared: (None, None),
            unresolved: &[
                ("fn f() { undefined_name(); }\n", "undefined_name", true),
                ("fn f() { Vec::new(); }\n", "Vec", false),
                ("use m::*;\nfn f() { g(); }\n", "g", false),
            ],
            stubs: false,
            redefines_shadowed: false,
            reference_kinds: &[
                "identifier",
                "type_identifier",
                "self",
                "shorthand_field_identifier",
                "lifetime",
                "label",
            ],
            name_kinds: &[
                "identifier",
                "type_identifier",
                "self",
                "shorthand_field_identifier",
                "lifetime",
                "label",
            ],
            resolves: (
                "fn f() {\n    let x = 1;\n    x;\n}\n",
                "x",
                Some(crate::code_rule_dsl::scope::Form::Assignment),
            ),
            refuses: &[
                (
                    super::ScopeFeature::This,
                    "`this` asks whether JavaScript's `this` is the global object; Rust has none (`self` is a method's parameter, which `binding` reads)",
                ),
                (
                    super::ScopeFeature::BeforeLocal,
                    "`beforeLocal` reads Python's binding order; a Rust function sees no local of an enclosing one, which `binding` reads",
                ),
            ],
            reads: &[
                super::ScopeFeature::Imports,
                super::ScopeFeature::Every,
                super::ScopeFeature::Occurrence,
            ],
            unpackaged: None,
        },
    });
}

/// Whether `A` reads `semantic`'s concern.
fn reads<A: Adapter>(semantic: Semantic) -> bool {
    match semantic {
        Semantic::Scope => <A::Scope as ScopeRules>::SUPPORTED,
        Semantic::Value => <A::Value as ValueRules>::SUPPORTED,
        Semantic::Arguments => <A::Arguments as ArgumentRules>::SUPPORTED,
        Semantic::Calls => <A::Calls as CallRules>::SUPPORTED,
        Semantic::FormatArgs => <A::FormatArgs as FormatArgRules>::SUPPORTED,
    }
}

/// A language's registry entry supports a capability exactly where its family's adapter reads
/// the concern.
#[test]
fn adapters_read_the_capabilities_the_registry_supports() {
    for language in CodeRuleLanguage::ALL {
        for semantic in [
            Semantic::Scope,
            Semantic::Value,
            Semantic::Arguments,
            Semantic::Calls,
            Semantic::FormatArgs,
        ] {
            assert_eq!(
                language.capability(semantic) == Support::Supported,
                with_adapter!(language.family(), A => reads::<A>(semantic)),
                "{} {semantic:?}",
                language.name()
            );
        }
    }
}

/// The scope kinds, and the kinds the shared walks pass over, are the ones the shared code named
/// before they moved here, in every registry dialect.
#[test]
fn shared_walks_read_the_kinds_they_named() {
    fn scope_kinds<S: ScopeRules>(dialect: GrammarDialect) {
        let ids = |names: &[&str]| kind_ids(dialect, names);
        assert_eq!(
            ids(S::PAREN_KINDS),
            ids(&["parenthesized_expression"]),
            "{dialect:?}"
        );
        if S::FLOW {
            return;
        }
        assert_eq!(
            ids(S::DOTTED_NAME_KINDS),
            ids(&["dotted_name"]),
            "{dialect:?}"
        );
        assert_eq!(ids(S::THIS_KINDS), ids(&["this"]), "{dialect:?}");
        let attribute = if S::IMPORT_PATHS {
            ids(&["scoped_identifier", "scoped_type_identifier"])
        } else {
            ids(&["attribute"])
        };
        assert_eq!(ids(S::ATTRIBUTE_KINDS), attribute, "{dialect:?}");
    }
    for language in CodeRuleLanguage::ALL {
        for dialect in language.dialects().iter().copied() {
            with_adapter!(language.family(), A => scope_kinds::<<A as Adapter>::Scope>(dialect));
            for name in (0..dialect.kind_count() as u16).filter_map(|id| dialect.kind_name(id)) {
                assert_eq!(
                    is_comment_kind(name),
                    matches!(
                        name,
                        "comment" | "html_comment" | "line_comment" | "block_comment"
                    ),
                    "{name} in {dialect:?}"
                );
                assert_eq!(is_paren_kind(name), name == "parenthesized_expression");
            }
        }
    }
}

/// A file is parsed by Forte, and may ask for import targets, in the families that did before
/// these moved onto the adapter.
#[test]
fn family_choices_stay_with_their_families() {
    for language in CodeRuleLanguage::ALL {
        for dialect in language.dialects().iter().copied() {
            assert_eq!(parsed_by_forte(dialect), dialect == GrammarDialect::Blade);
            assert_eq!(
                has_import_targets(dialect),
                Family::of(dialect) == Family::Ecma
            );
        }
    }
}
