//! The adapter test kit: every family's adapter against known-good literals (`kit`), and the
//! registry's capabilities against the concerns the adapters read.

use super::value::kit as value_kit;
use super::{
    Adapter, ArgumentRules, Blade, CallRules, Ecma, FormatArgRules, Php, Python, ScopeRules,
    ValueRules, with_adapter,
};
use crate::code_rule_dsl::language::{CodeRuleLanguage, Family, GrammarDialect, Semantic, Support};
use crate::code_rule_dsl::lines::Terminators;
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::value::{Lang, Value};
use ast_grep_core::AstGrep;
use ast_grep_core::tree_sitter::StrDoc;
use std::borrow::Cow;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

/// A document, and the kind and range of each of its regions' roots.
type Document = (&'static str, &'static [(&'static str, Range<usize>)]);

/// What one family's adapter answers.
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
}

/// A tape of `source` in `dialect`, as files are read.
fn tape(source: &str, dialect: GrammarDialect) -> AstGrep<TapeDoc> {
    let grammar = Grammar::new(dialect).expect("grammar");
    let native = StrDoc::new(source, dialect);
    let image =
        TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).expect("encodes");
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
        let read = A::regions(&root)
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
    value_kit::check::<A>(&expect.value, expect.dialect, tape);
    super::calls::kit::check::<A>(&expect.calls, expect.dialect);
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
            lang: Lang::Python,
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
            lang: Lang::Js,
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
            refusal: Some("calls follows Python and PHP calls only"),
            seed_language: None,
            taped: false,
            subclasses: false,
            summary: ("function f() { g(); }\n", &[]),
            unread: None,
            label: ("pkg.f", "pkg.f"),
            keys: &[("pkg.f", 0, &[])],
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
            lang: Lang::Php,
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
            lang: Lang::Php,
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
