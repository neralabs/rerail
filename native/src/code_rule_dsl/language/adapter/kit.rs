//! The adapter test kit: every family's adapter against known-good literals (`kit`), and the
//! registry's capabilities against the concerns the adapters read. A new family adds one `#[test]`
//! function calling `kit::<Family>(&Expect { .. })`.

use super::value::kit as value_kit;
use super::{
    Adapter, ArgumentRules, Blade, CallRules, Ecma, FormatArgRules, Php, Python, Rust, ScopeRules,
    Sql, ValueRules, with_adapter,
};
use crate::code_rule_dsl::language::{
    CodeRuleLanguage, Family, GrammarDialect, Semantic, SourceType, Support,
};
use crate::code_rule_dsl::lines::Terminators;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::value::Value;
use ast_grep_core::AstGrep;
use std::borrow::Cow;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

/// A document, and the kind and range of each of its regions' roots.
type Document = (&'static str, &'static [(&'static str, Range<usize>)]);

/// What one family's adapter answers: the expectations `kit` checks. `family`, `label` and
/// `terminators` identify the adapter; `dialect` is the grammar its documents are read in; `name` is
/// what the identifier `ﬁle` normalises to; `regions` lists each document with the kind and range of
/// its regions' roots; `module` is the family's file suffix and the module of `pkg/mod<suffix>`; and
/// `value`, `calls`, `arguments`, `format_args`, `syntax` and `scope` are the expectations of each
/// concern's kit.
struct Expect {
    family: Family,
    label: &'static str,
    terminators: Terminators,
    /// The dialect the family's documents are read in.
    dialect: GrammarDialect,
    /// The name of the identifier `ﬁle`.
    name: &'static str,
    regions: &'static [Document],
    /// The suffix of the family's files, and the module of `pkg/mod<suffix>` in a package `pkg`.
    module: (&'static str, Option<&'static str>),
    value: value_kit::Expect,
    /// What the family's calls answer.
    calls: super::calls::kit::Calls,
    arguments: super::arguments::kit::Expect,
    format_args: super::format_args::kit::Expect,
    syntax: super::syntax::kit::Expect,
    scope: super::scope::kit::Expect,
}

/// A tape of `source` in `dialect`, as files are read: Blade's from its Forte tree, the rest from
/// tree-sitter's.
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
/// `terminators`), name normalisation, the regions of each document, module paths, and each
/// concern's own kit. A new family adds one `#[test]` function calling `kit::<Family>(&Expect { .. })`.
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
    for (source, regions) in expect.regions {
        let doc = tape(source, expect.dialect);
        let root = doc.root();
        let read = super::regions(&root, expect.dialect)
            .map(|region| (region.root.kind().into_owned(), region.root.range()))
            .collect::<Vec<_>>();
        let expected = regions
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
    value_kit::check::<A>(&expect.value, expect.dialect, tape);
    super::calls::kit::check::<A>(&expect.calls, expect.dialect);
    super::arguments::kit::check::<A>(&expect.arguments, expect.dialect, |s| {
        tape(s, expect.dialect)
    });
    super::format_args::kit::check::<A>(&expect.format_args);
    with_adapter!(expect.family, B => super::syntax::kit::check::<B>(&expect.syntax, expect.dialect, |s| tape(s, expect.dialect)));
    super::scope::kit::kit::<A>(expect.dialect, expect.module.0, &expect.scope);
}

#[test]
fn python() {
    kit::<Python>(&Expect {
        family: Family::Python,
        label: "python",
        terminators: Terminators::Newlines,
        dialect: GrammarDialect::Python,
        name: "file",
        regions: &[("x = 1\n", &[("module", 0..6)])],
        module: (".py", Some("pkg.mod")),
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
            taped: false,
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
                (r#"{"pythonVersion": {"min": "3.10"}}"#, Ok(true)),
                (r#"{"pythonVersion": {"max": "3.9"}}"#, Ok(false)),
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
            embeds: &[("sprintf($F, $$$A)", false)],
            php_comments: ("# c\nx = 1\n", 1),
        },
        scope: super::scope::kit::Expect {
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
        name: "ﬁle",
        regions: &[("let x = 1;\n", &[("program", 0..11)])],
        module: (".ts", None),
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
            taped: false,
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
                (r#"{"ecmaVersion": {"min": 2015}}"#, Ok(true)),
                (r#"{"sourceType": "script"}"#, Ok(false)),
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
            embeds: &[("sprintf($F, $$$A)", false)],
            php_comments: ("// c\nlet x = 1;\n", 1),
        },
        scope: super::scope::kit::Expect {
            flow: false,
            declared: (Some((2026, SourceType::Module)), None),
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
        name: "ﬁle",
        regions: &[("<?php $x = 1;\n", &[("program", 0..14)])],
        module: (".php", None),
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
            taped: false,
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
                (r#"{"ecmaVersion": {"min": 2015}}"#, Ok(true)),
                (r#"{"sourceType": "script"}"#, Ok(false)),
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
            embeds: &[("sprintf($F, $$$A)", false)],
            php_comments: ("<?php /** d */ // c\n", 2),
        },
        scope: super::scope::kit::Expect {
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
        name: "ﬁle",
        regions: &[
            ("<p>{{ $x }}</p>\n", &[("program", 3..11)]),
            ("<p>x</p>\n", &[]),
        ],
        module: (".blade.php", None),
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
                "`structure-parts` compares Python's implicitly concatenated string parts; blade embeds PHP, which has no implicit string concatenation",
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
            taped: true,
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
                    "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; blade has none (use language or syntactic)",
                ),
            ],
            passed: ("{{ f(1, b: 2) }}\n", ["1", "2"]),
        },
        format_args: super::format_args::kit::Expect {
            follows_strings: true,
            styles: [
                Some(
                    "`brace` reads Python's `str.format` calls; blade embeds PHP, which has no such templates (its printf-family calls are style `printf`)",
                ),
                Some(
                    "`percent` reads Python's `%` formats; blade embeds PHP, whose printf-family calls are style `printf`",
                ),
                None,
                None,
                Some(
                    "`rust` reads Rust's format macros; blade embeds PHP, whose printf-family calls are style `printf`",
                ),
            ],
        },
        syntax: super::syntax::kit::Expect {
            dollar_names: false,
            language_options: Some("`languageOptions` is not available for blade policies"),
            compiled_options: None,
            when: &[
                (r#"{"ecmaVersion": {"min": 2015}}"#, Ok(true)),
                (r#"{"sourceType": "script"}"#, Ok(false)),
                (
                    r#"{"pythonVersion": {"min": "3.10"}}"#,
                    Err("unknown field `pythonVersion` at /q/when"),
                ),
            ],
            parse: [
                Some(
                    "`percentFormat` parses Python literals; blade embeds PHP, whose printf formats are `printfFormat`",
                ),
                Some(
                    "`braceFormat` parses Python literals; blade embeds PHP, which has no str.format templates (its printf formats are `printfFormat`)",
                ),
                Some(
                    "`annotation` parses Python literals; blade embeds PHP, which holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
                ),
                Some("`eslintDirective` parses TypeScript comments"),
                None,
                None,
                None,
                Some("`rustFormat` parses Rust strings"),
            ],
            embeds: &[("sprintf($F, $$$A)", true), ("<p>{{ $x }}</p>", false)],
            php_comments: ("{{-- a --}}\n@php /** b */ @endphp\n", 1),
        },
        scope: super::scope::kit::Expect {
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
                    "blade policies ask `unresolved: true`",
                ),
                (
                    super::ScopeFeature::SameImport,
                    "qualified import names are Python's; a blade variable is never bound by an import",
                ),
            ],
            reads: &[super::ScopeFeature::Imports, super::ScopeFeature::Every],
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
        name: "ﬁle",
        regions: &[("SELECT 1;\n", &[("program", 0..10)])],
        module: (".sql", None),
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
            taped: false,
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
                ("{}", Ok(true)),
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
            embeds: &[("SELECT $A", false)],
            php_comments: ("-- c\nSELECT 1;\n", 1),
        },
        scope: super::scope::kit::Expect {
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
        name: "ﬁle",
        regions: &[("fn f() {}\n", &[("source_file", 0..10)])],
        module: (".rs", None),
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
            taped: false,
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
                (r#"{"ecmaVersion": {"min": 2015}}"#, Ok(true)),
                (r#"{"sourceType": "script"}"#, Ok(false)),
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
            embeds: &[("println!($F, $$$A)", false)],
            php_comments: ("// c\nfn f() {}\n", 0),
        },
        scope: super::scope::kit::Expect {
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
